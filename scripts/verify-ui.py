#!/usr/bin/env python3
"""Run isolated desktop UI and persistence checks on the remote Linux worker.

This harness never opens the operator's normal application data. Screenshots are
desktop evidence and do not establish Kindle panel, input, or firmware support.
Only explicitly supplied golden images participate in pixel baseline checks.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import dataclasses
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
from typing import Any, Callable, Iterable
from urllib.parse import quote

from PIL import Image, ImageChops


PROFILES = {
    "pw5": (618, 824),
    "pw6": (632, 840),
    "scribe": (930, 1240),
    "scribe2025": (990, 1320),
}
SCREENS = ("login", "library", "search", "detail", "reader", "mp", "article", "stats", "settings")
THOUGHT = "UI regression thought"
CLIPBOARD_QUOTE = "\u4e2d\u65e0\u6742\u6811"
IMPORT_TITLE = "Verification Fixture"
IMPORT_BODY = (
    "Chapter One\n"
    "This local text was created by the UI verification harness.\n"
    "It contains no account data or downloaded book content.\n\n"
    "Chapter Two\n"
    "A second chapter makes the import and catalog checks independent of a cover.\n"
)


def action(at_ms: int, section: str, name: str, value: str = "") -> dict[str, Any]:
    return {"at_ms": at_ms, "section": section, "action": name, "value": value}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def redact(value: str) -> str:
    value = re.sub(
        r"(?i)\b(bearer|api[_-]?key|authorization|cookie|wr_skey|wr_vid|wr_token|access_token)\s*[:= ]\s*[^\s,;]+",
        r"\1=[redacted]",
        value,
    )
    return value[-100_000:]


@dataclasses.dataclass
class Result:
    name: str
    passed: bool
    exit_code: int | None = None
    duration_seconds: float = 0.0
    screenshots: list[str] = dataclasses.field(default_factory=list)
    checks: dict[str, Any] = dataclasses.field(default_factory=dict)
    error: str | None = None
    log: str | None = None


@dataclasses.dataclass
class Capture:
    path: Path
    database: Path
    metadata: dict[str, Any]
    data_directory: Path
    state: dict[str, Any] = dataclasses.field(default_factory=dict)


class CaseFailure(RuntimeError):
    pass


class Verifier:
    def __init__(self, arguments: argparse.Namespace) -> None:
        self.arguments = arguments
        self.repository = Path(__file__).resolve().parents[1]
        self.labels = json.loads((self.repository / "assets/i18n/zh-CN.json").read_text(encoding="utf-8-sig"))
        self.binary = arguments.binary.resolve()
        require(self.binary.is_file(), f"Application binary does not exist: {self.binary}")
        self.binary_sha256 = hashlib.sha256(self.binary.read_bytes()).hexdigest()
        binary_stat = self.binary.stat()
        self.binary_identity = (binary_stat.st_ino, binary_stat.st_size, binary_stat.st_mtime_ns)
        stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        self.output = arguments.output_dir.resolve() / f"run-{stamp}-{os.getpid()}"
        self.output.mkdir(parents=True, exist_ok=False)
        if arguments.data_dir is None:
            self.data_root = Path(tempfile.mkdtemp(prefix="weread-ui-verification-"))
        else:
            self.data_root = arguments.data_dir.resolve()
            if self.data_root.exists():
                require(not any(self.data_root.iterdir()), "The verification data directory must be empty")
            else:
                self.data_root.mkdir(parents=True)
        (self.data_root / "verification-owner.json").write_text(
            json.dumps({"purpose": "isolated UI verification", "created_at": stamp}, indent=2),
            encoding="utf-8",
        )
        self.results: list[Result] = []
        self.lock = threading.Lock()
        self.environment = os.environ.copy()
        for key in list(self.environment):
            if key.startswith("WEREAD_"):
                del self.environment[key]
        self.environment["SLINT_SCALE_FACTOR"] = "1"
        self.environment["RUST_BACKTRACE"] = "1"

    def record(self, result: Result) -> None:
        with self.lock:
            self.results.append(result)
            state = "PASS" if result.passed else "FAIL"
            print(f"{state} {result.name}" + (f": {result.error}" if result.error else ""), flush=True)

    def state_directory(self, key: str) -> Path:
        path = self.data_root / key
        path.mkdir(parents=True, exist_ok=True)
        require(path.resolve().is_relative_to(self.data_root), "A case data path escaped the isolated root")
        return path

    def run_case(
        self,
        name: str,
        *,
        profile: str = "pw6",
        screen: str = "library",
        actions: Iterable[dict[str, Any]] = (),
        data_key: str | None = None,
        expected_route: str | None = None,
        demo: bool = True,
        exit_ms: int | None = None,
        runner: Path | None = None,
        runner_arguments: Iterable[str] = (),
    ) -> Capture:
        actions = sorted(actions, key=lambda item: item["at_ms"])
        if runner is None:
            binary_stat = self.binary.stat()
            require(
                (binary_stat.st_ino, binary_stat.st_size, binary_stat.st_mtime_ns) == self.binary_identity,
                "The application binary changed during verification; start a fresh run against the rebuilt binary",
            )
        directory = self.state_directory(data_key or name)
        screenshot = self.output / f"{name}.png"
        screenshot.parent.mkdir(parents=True, exist_ok=True)
        log_path = self.output / "logs" / f"{name}.log"
        log_path.parent.mkdir(parents=True, exist_ok=True)
        command = [
            "xvfb-run", "-a", "-s", "-screen 0 2200x3000x24 -nolisten tcp",
            str(runner or self.binary), "--offline", "--device", profile,
            "--data-dir", str(directory), "--screen", screen,
            "--capture", str(screenshot),
        ]
        if demo:
            command.append("--demo")
        command.extend(runner_arguments)
        if actions:
            action_path = self.output / "actions" / f"{name}.json"
            action_path.parent.mkdir(parents=True, exist_ok=True)
            action_path.write_text(json.dumps(actions, ensure_ascii=False, indent=2), encoding="utf-8")
            command.extend(["--actions", str(action_path)])
        deadline = exit_ms or max(1_200, actions[-1]["at_ms"] + 900 if actions else 1_200)
        command.extend(["--exit-after-ms", str(deadline)])
        started = time.monotonic()
        result = Result(name=name, passed=False, log=str(log_path))
        try:
            process = subprocess.Popen(
                command,
                cwd=self.repository,
                env=self.environment,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True,
                errors="replace",
                start_new_session=True,
            )
            try:
                output, _ = process.communicate(timeout=max(40.0, deadline / 1000 + 25.0))
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                output, _ = process.communicate()
                result.exit_code = process.returncode
                log_path.write_text(redact(output), encoding="utf-8")
                raise AssertionError("Application or Xvfb exceeded the bounded case timeout")
            result.exit_code = process.returncode
            log_path.write_text(redact(output), encoding="utf-8")
            require(process.returncode == 0, f"Application exited with {process.returncode}; see {log_path}")
            require(screenshot.is_file(), "The application did not write the requested screenshot")
            result.screenshots.append(str(screenshot))
            metadata = json.loads(screenshot.with_suffix(".metadata.json").read_text(encoding="utf-8"))
            result.checks.update(self.check_capture(screenshot, metadata, profile, expected_route or screen))
            if self.arguments.baseline_dir is not None:
                baseline = self.arguments.baseline_dir.resolve() / f"{name}.png"
                require(baseline.is_file(), f"Requested golden image is missing: {baseline}")
                difference = pixel_difference(baseline, screenshot, threshold=8)
                result.checks["golden_difference_ratio"] = difference
                require(difference <= self.arguments.max_diff_ratio, f"Golden image differs by {difference:.4%}")
            database = directory / ("demo" if demo else "") / "library.sqlite"
            require(database.is_file(), "The application did not create its isolated metadata database")
            result.passed = True
            state_path = screenshot.with_suffix(".state.json")
            state = json.loads(state_path.read_text(encoding="utf-8")) if state_path.is_file() else {}
            return Capture(screenshot, database, metadata, directory, state)
        except Exception as error:
            result.error = redact(str(error))
            raise CaseFailure(f"{name}: {result.error}") from error
        finally:
            if screenshot.is_file() and str(screenshot) not in result.screenshots:
                result.screenshots.append(str(screenshot))
            result.duration_seconds = round(time.monotonic() - started, 3)
            self.record(result)

    def check_capture(self, path: Path, metadata: dict[str, Any], profile: str, route: str) -> dict[str, Any]:
        expected = PROFILES[profile]
        require(metadata.get("profile") == profile, "Capture profile does not match the requested device")
        require(metadata.get("route") == route, f"Expected route {route!r}, received {metadata.get('route')!r}")
        require(metadata.get("backend") == "desktop", "The UI harness must use the desktop backend")
        require(metadata.get("scale_factor") == 1.0, "Desktop captures must use the fixed logical scale")
        for key in ("window_size", "logical_size", "snapshot_size"):
            require(tuple(metadata.get(key, ())) == expected, f"{key} does not match the strict {expected} profile geometry")
        with Image.open(path) as source:
            require(source.size == expected, f"PNG dimensions {source.size} do not match {expected}")
            image = source.convert("RGB")
            red, green, blue = image.split()
            require(ImageChops.difference(red, green).getextrema()[1] <= 1, "A screenshot contains non-grayscale red/green pixels")
            require(ImageChops.difference(red, blue).getextrema()[1] <= 1, "A screenshot contains non-grayscale red/blue pixels")
            histogram = image.convert("L").histogram()
            total = image.width * image.height
            ink_ratio = sum(histogram[:64]) / total
            paper_ratio = sum(histogram[240:]) / total
            require(ink_ratio > 0.002, "Screenshot is blank or contains too little rendered text")
            require(paper_ratio > 0.05, "Screenshot has no usable paper background")
            require(ink_ratio < 0.9, "Screenshot appears to be a solid dark frame")
        return {
            "profile": profile,
            "route": route,
            "geometry": list(expected),
            "ink_ratio": round(ink_ratio, 6),
            "paper_ratio": round(paper_ratio, 6),
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
        }

    def assertion(self, name: str, check: Callable[[], dict[str, Any] | None], *captures: Capture) -> None:
        started = time.monotonic()
        result = Result(name=name, passed=False, screenshots=[str(item.path) for item in captures])
        try:
            result.checks.update(check() or {})
            result.passed = True
        except Exception as error:
            result.error = redact(str(error))
        result.duration_seconds = round(time.monotonic() - started, 3)
        self.record(result)

    def execute_group(self, name: str, callback: Callable[[], None]) -> None:
        try:
            callback()
        except CaseFailure:
            pass
        except Exception as error:
            self.record(Result(name=f"{name}/setup", passed=False, error=redact(str(error))))

    def snapshot(self, profile: str, screen: str) -> None:
        self.run_case(f"snapshots/{profile}/{screen}", profile=profile, screen=screen)

    def library_flow(self) -> None:
        key = "flows/library"
        cover = self.run_case(f"{key}/cover", data_key=key)
        listing = self.run_case(f"{key}/list", data_key=key, actions=[action(300, "library", "view")])

        def list_saved() -> dict[str, Any]:
            preferences = setting(listing.database, "preferences")
            require(preferences["view_mode"] == "list", "List view was not persisted")
            difference = pixel_difference(cover.path, listing.path)
            require(difference > 0.05, "The list action did not materially change the rendered library")
            return {"saved_view": preferences["view_mode"], "difference_ratio": difference}

        self.assertion(f"{key}/list-persisted", list_saved, cover, listing)
        second = self.run_case(f"{key}/page-two", data_key=key, actions=[action(300, "library", "page", "1")])
        self.assertion(f"{key}/page-rendered", lambda: changed(listing, second, 0.01), listing, second)
        downloaded = self.run_case(f"{key}/downloaded", data_key=key, actions=[action(300, "library", "view"), action(500, "library", "sort", "downloaded")])
        self.assertion(f"{key}/filter-rendered", lambda: changed(cover, downloaded, 0.02), cover, downloaded)
        selected = self.run_case(f"{key}/selected", data_key=key, actions=[action(300, "library", "select"), action(500, "library", "book", "demo-1")])
        self.assertion(f"{key}/selection-rendered", lambda: changed(cover, selected, 0.0015), cover, selected)
        removed = self.run_case(
            f"{key}/remove-confirmed", data_key=key,
            actions=[action(300, "library", "select"), action(500, "library", "book", "demo-1"), action(700, "shell", "remove-selected"), action(900, "shell", "dialog-primary")],
        )

        def removed_saved() -> dict[str, Any]:
            require(scalar(removed.database, "SELECT on_shelf FROM books WHERE id='demo-1'") == 0, "Confirmed removal did not change shelf membership")
            require(scalar(removed.database, "SELECT count(*) FROM chapters WHERE book_id='demo-0' AND length(body)>0") == 3, "Removing a shelf item damaged cached reading content")
            require(scalar(removed.database, "SELECT count(*) FROM download_tasks") == 0, "Offline UI created a fabricated download task")
            return {"removed_from_shelf": "demo-1", "cached_chapters_retained": 3}

        self.assertion(f"{key}/removal-persisted", removed_saved, removed)

    def detail_flow(self) -> None:
        key = "flows/detail"
        catalog = self.run_case(f"{key}/catalog", screen="detail", data_key=key)
        intro = self.run_case(f"{key}/intro", screen="detail", data_key=key, actions=[action(300, "detail", "tab", "intro")])
        self.assertion(f"{key}/tab-rendered", lambda: changed(catalog, intro, 0.01), catalog, intro)
        selection = self.run_case(f"{key}/selection", screen="detail", data_key=key, actions=[action(300, "detail", "select"), action(500, "detail", "select-page"), action(700, "pointer", "tap", "300,420")])
        self.assertion(f"{key}/selection-rendered", lambda: changed(catalog, selection, 0.0015), catalog, selection)
        self.assertion(f"{key}/cached-chapters-remain-complete", lambda: cached_chapters_intact(selection.database), selection)

    def search_flow(self) -> None:
        key = "flows/search"
        query = self.labels["sample_title"][:2]
        actions = [action(300, "search", "scope", "shelf")]
        actions.extend(action(500 + index * 160, "search", "key", key_name) for index, key_name in enumerate("guwen"))
        actions.extend([action(1_400, "search", "candidate", query), action(1_650, "search", "key", "submit")])
        result = self.run_case(f"{key}/pinyin", screen="search", data_key=key, actions=actions)

        def query_saved() -> dict[str, Any]:
            require(scalar(result.database, "SELECT count(*) FROM query_history WHERE query=?", (query,)) == 1, "Pinyin composition did not commit the requested Chinese search")
            require(scalar(result.database, "SELECT count(*) FROM query_history WHERE query='guwen'") == 0, "Uncommitted pinyin was saved as the search query")
            require(scalar(result.database, "SELECT count(*) FROM books WHERE title LIKE ? AND on_shelf=1", (f"%{query}%",)) >= 1, "The submitted query has no expected local fixture match")
            return {"committed_query": query}

        self.assertion(f"{key}/query-history", query_saved, result)
        restarted = self.run_case(f"{key}/history-after-restart", screen="search", data_key=key)
        self.assertion(f"{key}/history-retained", lambda: {"query_count": require_query(restarted.database, query)}, restarted)
        keyboard = self.run_case(f"{key}/initial-keyboard", screen="search", data_key=f"{key}/reentry")
        reopened = self.run_case(
            f"{key}/reopened-keyboard", screen="search", data_key=f"{key}/reentry",
            actions=[action(300, "search", "scope", "shelf"), action(500, "search", "edit-query", query), action(700, "search", "key", "submit"), action(900, "search", "back"), action(1_100, "library", "search", "shelf")],
        )
        def keyboard_restored() -> dict[str, Any]:
            difference = pixel_difference(keyboard.path, reopened.path, crop=(0, 540, 632, 840), threshold=8)
            require(difference <= 0.001, "Reopening book search did not restore its touch keyboard")
            return {"keyboard_difference_ratio": difference}
        self.assertion(f"{key}/entry-restores-touch-keyboard", keyboard_restored, keyboard, reopened)

    def pointer_flow(self) -> None:
        for route, fraction in (("mp", 3 / 8), ("stats", 5 / 8), ("settings", 7 / 8)):
            self.run_case(f"flows/pointer/tab-{route}", actions=[action(300, "pointer", "tap", f"{632 * fraction},808")], expected_route=route)
        self.run_case("flows/pointer/book-detail", actions=[action(300, "pointer", "tap", "110,310")], expected_route="detail")
        opened = self.run_case(
            "flows/pointer/book-detail-reader",
            actions=[action(300, "pointer", "tap", "110,310"), action(700, "pointer", "tap", "180,260")],
            expected_route="reader",
        )
        self.assertion("flows/pointer/reader-has-real-content", lambda: cached_chapters_intact(opened.database), opened)

    def reader_flow(self) -> None:
        key = "flows/reader"
        baseline = self.run_case(f"{key}/before", screen="reader", data_key=key)
        actions = [
            action(300, "reader", "bookmark"),
            action(500, "pointer", "move", "316,208"), action(600, "pointer", "down", "316,208"), action(1_400, "pointer", "up", "316,208"),
            action(1_700, "reader", "highlight"),
            action(2_000, "pointer", "move", "316,288"), action(2_100, "pointer", "down", "316,288"), action(2_900, "pointer", "up", "316,288"),
            action(3_200, "reader", "write-thought"), action(3_500, "search", "edit-query", THOUGHT), action(3_800, "search", "key", "submit"),
            action(4_100, "reader", "menu", "true"), action(4_300, "reader", "tool", "type"), action(4_500, "reader", "font-size", "1"), action(4_700, "reader", "font", "hei"), action(5_000, "reader", "menu", "false"),
        ]
        saved = self.run_case(f"{key}/saved", screen="reader", data_key=key, actions=actions, exit_ms=6_800)
        self.assertion(f"{key}/notes-bookmark-typography", lambda: reader_persistence(saved.database), saved)
        self.assertion(f"{key}/reflow-rendered", lambda: changed(baseline, saved, 0.01), baseline, saved)
        restarted = self.run_case(f"{key}/after-restart", screen="reader", data_key=key)
        self.assertion(f"{key}/retained-after-restart", lambda: reader_persistence(restarted.database), restarted)

        def same_body() -> dict[str, Any]:
            difference = pixel_difference(saved.path, restarted.path, crop=(32, 164, 600, 800), threshold=8)
            require(difference <= 0.001, f"Saved reader body changed after restart: {difference:.4%}")
            return {"body_difference_ratio": difference}

        self.assertion(f"{key}/restart-screenshot", same_body, saved, restarted)

    def reader_image_flow(self) -> None:
        for profile in ("pw6", "scribe"):
            key = f"flows/reader-images/{profile}"
            expected_size = 34 if profile == "scribe" else 32

            def open_image_page(stage: str, *, reflow: bool = False) -> tuple[Capture, dict[str, Any], dict[str, Any]]:
                for page in range(3):
                    intents = [action(300, "reader", "open-chapter", "demo-chapter-1")]
                    if reflow and page == 0:
                        intents.append(action(600, "reader", "font-size", "5"))
                    intents.extend(action(900 + turn * 200, "reader", "turn", "1") for turn in range(page))
                    capture = self.run_case(f"{key}/{stage}-page-{page}", profile=profile, screen="reader", data_key=key, actions=intents)
                    asset = book_image_asset(capture.database)
                    match = match_book_image(capture.path, Path(asset["path"]))
                    if match is not None:
                        return capture, asset, match
                raise AssertionError(f"The cached illustration was not rendered on the first three {stage} pages")

            original, original_asset, original_match = open_image_page("original")
            self.assertion(f"{key}/actual-bitmap-rendered", lambda: original_match, original)
            reflowed, reflowed_asset, reflowed_match = open_image_page("reflowed", reflow=True)

            def reflow_intact() -> dict[str, Any]:
                require(reflowed_asset["payload_sha256"] == original_asset["payload_sha256"], "Reflow rewrote or lost the cached bitmap")
                require(reflowed_asset["manifest_sha256"] == original_asset["manifest_sha256"], "Reflow changed the chapter image anchors")
                preferences = setting(reflowed.database, "reader.preferences:demo-0")
                require(preferences["font_size"] == expected_size, "Image reflow did not persist the requested device typography step")
                position = rows(reflowed.database, "SELECT chapter_index FROM positions WHERE book_id='demo-0'")
                require(position and position[0]["chapter_index"] == 1, "The image chapter position was not saved")
                difference = pixel_difference(original.path, reflowed.path)
                require(difference > 0.005, "Changing typography did not reflow the illustrated reading page")
                return {**reflowed_match, "font_size": expected_size, "reflow_difference_ratio": difference}

            self.assertion(f"{key}/reflow-preserves-bitmap-and-anchor", reflow_intact, original, reflowed)
            restarted = self.run_case(f"{key}/after-restart", profile=profile, screen="detail", data_key=key, actions=[action(300, "detail", "read")], expected_route="reader")

            def restart_intact() -> dict[str, Any]:
                asset = book_image_asset(restarted.database)
                require(asset["payload_sha256"] == original_asset["payload_sha256"], "Restart lost or rewrote the cached illustration")
                require(asset["manifest_sha256"] == original_asset["manifest_sha256"], "Restart changed the persisted image manifest")
                match = match_book_image(restarted.path, Path(asset["path"]))
                require(match is not None, "Reader handoff after restart did not restore the page containing the illustration")
                width, height = PROFILES[profile]
                difference = pixel_difference(reflowed.path, restarted.path, crop=(32, 52, width - 32, height - 48), threshold=8)
                require(difference <= 0.001, f"Illustrated body changed after restart: {difference:.4%}")
                return {**match, "body_difference_ratio": difference}

            self.assertion(f"{key}/restart-preserves-visible-bitmap", restart_intact, reflowed, restarted)

    def clipboard_flow(self) -> None:
        key = "flows/clipboard"
        # PW6's second measured line contains the four fixed source glyphs.
        # Hold the first glyph, then visit the next three after activation.
        select_quote = [
            action(300, "pointer", "move", "316,224"),
            action(400, "pointer", "down", "316,224"),
            action(1_000, "pointer", "move", "339,224"),
            action(1_100, "pointer", "move", "362,224"),
            action(1_200, "pointer", "move", "385,224"),
            action(1_300, "pointer", "up", "385,224"),
        ]
        self.run_case(f"{key}/selection", screen="reader", actions=select_quote)
        pasted = self.run_case(
            f"{key}/copied-pasted-saved", screen="reader", data_key=key,
            actions=[*select_quote, action(1_500, "pointer", "tap", "504,480"), action(1_800, "pointer", "tap", "316,480"), action(2_200, "pointer", "tap", "602,566"), action(2_600, "pointer", "tap", "566,804")],
            expected_route="reader", exit_ms=5_000,
        )
        self.assertion(f"{key}/actual-ui-clipboard-persisted", lambda: clipboard_persistence(pasted.database), pasted)
        restarted = self.run_case(f"{key}/after-restart", screen="detail", data_key=key, actions=[action(300, "detail", "read")], expected_route="reader")
        self.assertion(f"{key}/retained-after-restart", lambda: clipboard_persistence(restarted.database), restarted)

    def publication_flow(self) -> None:
        key = "flows/publications"
        accounts = self.run_case(f"{key}/accounts", screen="mp", data_key=key)
        articles = self.run_case(f"{key}/articles", screen="mp", data_key=key, actions=[action(300, "pointer", "tap", "260,214")])
        self.assertion(f"{key}/account-rendered", lambda: changed(accounts, articles, 0.005), accounts, articles)
        opened = self.run_case(f"{key}/article", screen="mp", data_key=key, actions=[action(300, "auxiliary", "mp.account", "demo-publication"), action(600, "pointer", "tap", "260,214")], expected_route="article")
        self.assertion(f"{key}/read-marker", lambda: {"article_marked_read": require_setting(opened.database, "article.read:demo-article-0", True)}, opened)
        returned = self.run_case(f"{key}/returned", screen="mp", data_key=key, actions=[action(300, "auxiliary", "mp.account", "demo-publication"), action(500, "auxiliary", "mp.article", "demo-article-0"), action(800, "article", "article.back")], expected_route="mp")
        self.assertion(f"{key}/return-context", lambda: changed(accounts, returned, 0.005), accounts, returned)
        self.run_case(f"{key}/touch-keyboard", screen="mp", actions=[action(300, "pointer", "tap", "120,52")], expected_route="search")
        query = self.labels["sample_title"][:2]
        query_actions = [action(300, "pointer", "tap", "120,52")]
        query_actions.extend(action(500 + index * 160, "search", "key", key_name) for index, key_name in enumerate("guwen"))
        query_actions.extend([action(1_400, "search", "candidate", query), action(1_650, "search", "key", "submit")])
        queried = self.run_case(f"{key}/query-returned", screen="mp", actions=query_actions, expected_route="mp")
        def publication_query_isolated() -> dict[str, Any]:
            count = scalar(queried.database, "SELECT count(*) FROM query_history")
            require(count == 0, "Publication-purpose input leaked into ordinary book search history")
            return {"book_query_history_count": count}
        self.assertion(f"{key}/query-purpose-isolated", publication_query_isolated, queried)

    def article_flow(self) -> None:
        key = "flows/article"
        baseline = self.run_case(f"{key}/before", screen="article", data_key=key)
        saved = self.run_case(
            f"{key}/typography", screen="article", data_key=key,
            actions=[action(300, "article", "article.tool", "type"), action(500, "article", "article.font-size", "1"), action(700, "article", "article.font", "song"), action(900, "article", "article.weight", "700"), action(1_100, "article", "article.paragraph-gap", "loose"), action(1_300, "article", "article.star"), action(1_600, "article", "article.menu", "false")],
            exit_ms=3_800,
        )
        self.assertion(f"{key}/typography-and-local-star", lambda: article_persistence(saved.database), saved)
        self.assertion(f"{key}/reflow-rendered", lambda: changed(baseline, saved, 0.01), baseline, saved)
        restarted = self.run_case(f"{key}/after-restart", screen="article", data_key=key)
        self.assertion(f"{key}/retained-after-restart", lambda: article_persistence(restarted.database), restarted)

    def settings_flow(self) -> None:
        key = "flows/settings"
        saved = self.run_case(
            f"{key}/controls", screen="settings", data_key=key,
            actions=[
                action(300, "auxiliary", "settings.open", "sync"), action(500, "auxiliary", "settings.toggle", "report_time"),
                action(700, "auxiliary", "settings.open", "notes"), action(900, "auxiliary", "settings.toggle", "popular"), action(1_100, "auxiliary", "settings.increase", "edge_region"), action(1_300, "auxiliary", "settings.increase", "popup_height"),
                action(1_500, "auxiliary", "settings.open", "storage"), action(1_700, "auxiliary", "settings.toggle", "download_article_images"),
                action(1_900, "auxiliary", "settings.increase", "download_concurrency"), action(2_100, "auxiliary", "settings.increase", "download_concurrency"), action(2_300, "auxiliary", "settings.decrease", "download_concurrency"),
                action(2_500, "auxiliary", "settings.page", "1"), action(2_700, "auxiliary", "settings.page", "-1"), action(2_900, "auxiliary", "settings.open", "storage"),
            ],
        )
        self.assertion(f"{key}/controls-persisted", lambda: settings_persistence(saved.database), saved)
        restarted = self.run_case(f"{key}/after-restart", screen="settings", data_key=key, actions=[action(300, "auxiliary", "settings.open", "notes")])
        self.assertion(f"{key}/retained-after-restart", lambda: settings_persistence(restarted.database), restarted)
        for profile in ("pw6", "scribe"):
            for subpage in ("sync", "notes", "storage", "display", "device", "about"):
                self.run_case(f"{key}/{profile}-{subpage}", profile=profile, screen="settings", data_key=f"{key}/{profile}-subpages", actions=[action(300, "auxiliary", "settings.open", subpage)])

    def statistics_flow(self) -> None:
        key = "flows/statistics"
        baselines: dict[str, Capture] = {}
        for profile in ("pw6", "scribe"):
            baselines[profile] = self.run_case(f"{key}/{profile}-week", profile=profile, screen="stats")
            for period in ("month", "year", "total"):
                capture = self.run_case(f"{key}/{profile}-{period}", profile=profile, screen="stats", actions=[action(300, "auxiliary", "stats.period", period)])
                def period_changed(before: Capture = baselines[profile], after: Capture = capture, selected_profile: str = profile) -> dict[str, Any]:
                    width, height = PROFILES[selected_profile]
                    difference = pixel_difference(before.path, after.path, crop=(20, 28, width - 20, height - 80))
                    require(difference > 0.005, "Changing the statistics period did not change its rendered controls or data")
                    return {"period_difference_ratio": difference}
                self.assertion(f"{key}/{profile}-{period}-rendered", period_changed, baselines[profile], capture)
        chart_baseline = self.run_case(f"{key}/chart-before-selection", screen="stats", actions=[action(300, "auxiliary", "stats.previous")])
        selected = self.run_case(f"{key}/chart-selected", screen="stats", actions=[action(300, "auxiliary", "stats.previous"), action(600, "pointer", "tap", "60,410"), action(900, "auxiliary", "stats.bar-next")])
        self.assertion(f"{key}/uses-local-reading-records", lambda: {"reading_seconds": require_reading_sessions(selected.database)}, selected)
        def chart_changed() -> dict[str, Any]:
            difference = pixel_difference(chart_baseline.path, selected.path, crop=(20, 288, 612, 528))
            require(difference > 0.0005, "Pointer and step navigation did not change the selected statistics value")
            return {"chart_selection_difference_ratio": difference}
        self.assertion(f"{key}/chart-selection-rendered", chart_changed, chart_baseline, selected)

    def import_flow(self) -> None:
        key = "flows/import"
        directory = self.state_directory(key)
        imports = directory / "imports"
        imports.mkdir(parents=True, exist_ok=True)
        (imports / f"{IMPORT_TITLE}.txt").write_text(IMPORT_BODY, encoding="utf-8")
        imported = self.run_case(f"{key}/scanned", data_key=key, demo=False, actions=[action(300, "library", "import")])
        self.assertion(f"{key}/book-and-body-imported", lambda: imported_book(imported.database), imported)
        searched = self.run_case(f"{key}/searchable", screen="search", data_key=key, demo=False, actions=[action(300, "search", "scope", "shelf"), action(500, "search", "edit-query", IMPORT_TITLE), action(800, "search", "key", "submit")])
        self.assertion(f"{key}/query-recorded", lambda: {"query_count": require_query(searched.database, IMPORT_TITLE)}, searched)
        restarted = self.run_case(f"{key}/after-restart", data_key=key, demo=False)
        self.assertion(f"{key}/retained-after-restart", lambda: imported_book(restarted.database), restarted)

    def tasks(self) -> list[tuple[str, Callable[[], None]]]:
        tasks: list[tuple[str, Callable[[], None]]] = []
        for profile in ("pw6", "scribe"):
            for screen in SCREENS:
                tasks.append((f"snapshots/{profile}/{screen}", lambda p=profile, s=screen: self.snapshot(p, s)))
        for profile in ("pw5", "scribe2025"):
            for screen in ("library", "reader"):
                tasks.append((f"snapshots/{profile}/{screen}", lambda p=profile, s=screen: self.snapshot(p, s)))
        tasks.extend([
            ("flows/library", self.library_flow), ("flows/detail", self.detail_flow),
            ("flows/search", self.search_flow), ("flows/pointer", self.pointer_flow),
            ("flows/reader", self.reader_flow), ("flows/publications", self.publication_flow),
            ("flows/reader-images", self.reader_image_flow),
            ("flows/clipboard", self.clipboard_flow),
            ("flows/article", self.article_flow), ("flows/settings", self.settings_flow),
            ("flows/statistics", self.statistics_flow), ("flows/import", self.import_flow),
        ])
        if self.arguments.only:
            prefixes = tuple(part.strip().strip("/") for part in self.arguments.only.split(",") if part.strip())
            tasks = [(name, callback) for name, callback in tasks if any(name == prefix or name.startswith(prefix + "/") for prefix in prefixes)]
        require(bool(tasks), "No cases match the requested filter")
        return tasks

    def run(self) -> int:
        started = time.monotonic()
        with concurrent.futures.ThreadPoolExecutor(max_workers=self.arguments.workers) as executor:
            futures = [executor.submit(self.execute_group, name, callback) for name, callback in self.tasks()]
            for future in concurrent.futures.as_completed(futures):
                future.result()
        results = sorted(self.results, key=lambda result: result.name)
        report = {
            "environment": "remote Linux desktop with Xvfb; no Kindle device execution",
            "binary": str(self.binary),
            "binary_sha256": self.binary_sha256,
            "data_directory": str(self.data_root),
            "duration_seconds": round(time.monotonic() - started, 3),
            "passed": sum(result.passed for result in results),
            "failed": sum(not result.passed for result in results),
            "golden_comparison": self.arguments.baseline_dir is not None,
            "coverage_limits": [
                "Desktop captures do not verify Kindle hardware or physical panel behavior.",
                "Offline fixtures do not verify authenticated upstream operations.",
                "Current settings groups fit one page; page intents cover boundary clamping, not a multi-page fixture.",
            ],
            "cases": [dataclasses.asdict(result) for result in results],
        }
        report_path = self.output / "report.json"
        report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
        print(json.dumps({"report": str(report_path), "passed": report["passed"], "failed": report["failed"]}), flush=True)
        return 0 if report["failed"] == 0 else 1


def rows(database: Path, sql: str, parameters: tuple[Any, ...] = ()) -> list[sqlite3.Row]:
    require(database.is_file(), f"Metadata database is missing: {database}")
    uri = f"file:{quote(database.as_posix(), safe='/')}?mode=ro"
    with sqlite3.connect(uri, uri=True) as connection:
        connection.row_factory = sqlite3.Row
        return connection.execute(sql, parameters).fetchall()


def scalar(database: Path, sql: str, parameters: tuple[Any, ...] = ()) -> Any:
    values = rows(database, sql, parameters)
    require(bool(values), f"The expected metadata row is missing: {sql}")
    return values[0][0]


def setting(database: Path, key: str) -> Any:
    return json.loads(scalar(database, "SELECT value FROM settings WHERE key=?", (key,)))


def require_setting(database: Path, key: str, expected: Any) -> Any:
    value = setting(database, key)
    require(value == expected, f"Setting {key!r} is {value!r}, expected {expected!r}")
    return value


def require_query(database: Path, query: str) -> int:
    count = scalar(database, "SELECT count(*) FROM query_history WHERE query=?", (query,))
    require(count == 1, "The submitted search was not retained across application restarts")
    return count


def require_reading_sessions(database: Path) -> int:
    seconds = scalar(database, "SELECT coalesce(sum(seconds),0) FROM reading_sessions")
    require(seconds > 0, "Reading statistics have no independently stored source records")
    return seconds


def cached_chapters_intact(database: Path) -> dict[str, Any]:
    count = scalar(database, "SELECT count(*) FROM chapters WHERE book_id='demo-0' AND length(body)>0")
    require(count == 3, "Expected three complete public-domain demonstration chapters")
    require(scalar(database, "SELECT cached_chapters FROM books WHERE id='demo-0'") == count, "Cached chapter metadata disagrees with complete bodies")
    return {"complete_chapters": count}


def reader_persistence(database: Path) -> dict[str, Any]:
    notes = rows(database, "SELECT * FROM annotations WHERE book_id='demo-0' ORDER BY start_offset")
    require(len(notes) == 2, f"Two distinct long-pressed passages were expected, found {len(notes)}")
    require(sum(note["note"] == "" for note in notes) == 1, "The plain highlight was not saved")
    require(sum(note["note"] == THOUGHT for note in notes) == 1, "The Search-purpose thought was not saved exactly")
    for note in notes:
        body = scalar(database, "SELECT body FROM chapters WHERE book_id=? AND chapter_index=?", (note["book_id"], note["chapter_index"]))
        require(0 <= note["start_offset"] < note["end_offset"] <= len(body), "A saved annotation has invalid Unicode scalar offsets")
        require(body[note["start_offset"]:note["end_offset"]] == note["quote"], "Saved annotation offsets do not select its actual chapter quote")
    bookmarks = rows(database, "SELECT * FROM bookmarks WHERE book_id='demo-0'")
    require(len(bookmarks) == 1, "The bookmark was not saved exactly once")
    body = scalar(database, "SELECT body FROM chapters WHERE book_id='demo-0' AND chapter_index=?", (bookmarks[0]["chapter_index"],))
    require(0 <= bookmarks[0]["char_offset"] <= len(body), "Bookmark offset is outside its chapter")
    preferences = setting(database, "reader.preferences:demo-0")
    require(preferences["font_size"] == 24 and preferences["font"] == "hei" and preferences["book_only"] is True, "Book typography was not persisted independently")
    require(scalar(database, "SELECT count(*) FROM query_history WHERE query=?", (THOUGHT,)) == 0, "Thought input leaked into ordinary search history")
    reading_seconds = scalar(database, "SELECT coalesce(sum(seconds),0) FROM reading_sessions WHERE book_id='demo-0' AND id NOT LIKE 'demo-session-%'")
    require(reading_seconds > 0, "Actual reading time was not persisted outside the seeded statistics fixtures")
    return {"annotations": len(notes), "bookmarks": len(bookmarks), "reader_font": preferences["font"], "reader_font_size": preferences["font_size"], "actual_reading_seconds": reading_seconds}


def book_image_asset(database: Path) -> dict[str, Any]:
    manifest = setting(database, "chapter.assets:demo-0:demo-chapter-1")
    require(manifest["complete"] is True and manifest["unresolved_images"] == 0, "The real chapter image manifest is incomplete")
    require(len(manifest["images"]) == 1, "The illustrated fixture does not contain exactly one image occurrence")
    image = manifest["images"][0]
    body = scalar(database, "SELECT body FROM chapters WHERE book_id='demo-0' AND chapter_index=1")
    require(manifest["text_sha256"] == hashlib.sha256(body.encode("utf-8")).hexdigest(), "Image anchors are not bound to the exact canonical chapter text")
    require(isinstance(image["anchor"], int) and 0 < image["anchor"] < len(body), "The illustration has no valid Unicode scalar insertion anchor")
    require(image["source"] == "demo-illustration.png", "The committed image has the wrong fixture source identity")
    directory = database.parent / "images"
    entries = rows(directory / "cache-index.sqlite3", "SELECT filename,size,pinned FROM cache_entries WHERE key=?", (image["cache_key"],))
    require(len(entries) == 1 and entries[0]["pinned"] == 1, "The referenced book illustration is not durably cached and pinned")
    path = directory / entries[0]["filename"]
    require(path.resolve().is_relative_to(directory.resolve()) and path.is_file(), "The illustration payload is missing or outside its cache")
    require(path.stat().st_size == entries[0]["size"], "The cached illustration size disagrees with its index")
    with Image.open(path) as source:
        require(source.size == (960, 540), "The prepared local illustration has incorrect dimensions")
        grayscale = source.convert("L")
        require(source.mode == "L", "The canonical illustration is not an eight-bit grayscale PNG")
        require(all(count == 0 or value % 17 == 0 for value, count in enumerate(grayscale.histogram())), "The intentionally sixteen-tone demo source changed")
        for point, expected in (((1, 1), 0), ((100, 50), 255), ((730, 100), 170), ((720, 300), 221), ((720, 500), 68)):
            require(grayscale.getpixel(point) == expected, "The cache contains a placeholder instead of the real geometric illustration")
    return {
        "path": str(path),
        "payload_sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
        "manifest_sha256": hashlib.sha256(json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode("utf-8")).hexdigest(),
        "anchor": image["anchor"],
    }


def longest_tone_run(image: Image.Image, tone: int = 68) -> tuple[int, int, int] | None:
    grayscale = image.convert("L")
    pixels = grayscale.tobytes()
    best: tuple[int, int, int] | None = None
    best_length = 150
    for y in range(grayscale.height):
        start = 0
        length = 0
        row = pixels[y * grayscale.width:(y + 1) * grayscale.width]
        for x, value in enumerate(row):
            if abs(value - tone) <= 1:
                if length == 0:
                    start = x
                length += 1
                if length >= best_length:
                    best = (start, y, length)
                    best_length = length
            else:
                length = 0
    return best


def match_book_image(screenshot: Path, payload: Path) -> dict[str, Any] | None:
    with Image.open(payload) as payload_source, Image.open(screenshot) as screenshot_source:
        source = payload_source.convert("L")
        frame = screenshot_source.convert("L")
        reference_run, observed_run = longest_tone_run(source), longest_tone_run(frame)
        require(reference_run is not None, "The illustration fixture has no distinctive grayscale region")
        if observed_run is None:
            return None
        reference_x, reference_y, reference_length = reference_run
        observed_x, observed_y, observed_length = observed_run
        scale = observed_length / reference_length
        width, height = round(source.width * scale), round(source.height * scale)
        if not (180 <= width <= 930 and 80 <= height <= 900):
            return None
        # Raster resampling can move the last flat-tone pixel by one source sample.
        # Register a bounded neighborhood before comparing the actual bitmap.
        best: tuple[float, float, list[int]] | None = None
        for candidate_width in range(max(1, width - 4), width + 5):
            candidate_height = round(source.height * candidate_width / source.width)
            candidate_scale = candidate_width / source.width
            expected = source.resize((candidate_width, candidate_height), Image.Resampling.BILINEAR)
            # These isolated cases retain the default Off policy. Canonical
            # Gray8 stays untouched on disk; quantization follows final sizing.
            expected = expected.point(lambda value: min(15, (value + 8) // 17) * 17)
            predicted_left = observed_x - round(reference_x * candidate_scale)
            predicted_top = observed_y - round(reference_y * candidate_scale)
            for dx in range(-2, 3):
                for dy in range(-2, 3):
                    left, top = predicted_left + dx, predicted_top + dy
                    if left < 0 or top < 0 or left + candidate_width > frame.width or top + candidate_height > frame.height:
                        continue
                    actual = frame.crop((left, top, left + candidate_width, top + candidate_height))
                    histogram = ImageChops.difference(expected, actual).histogram()
                    area = candidate_width * candidate_height
                    mean_error = sum(value * count for value, count in enumerate(histogram)) / area
                    difference_ratio = sum(histogram[21:]) / area
                    if best is None or mean_error < best[0]:
                        best = (mean_error, difference_ratio, [left, top, candidate_width, candidate_height])
        if best is None or best[0] > 2.0 or best[1] > 0.02:
            return None
        return {"bitmap_bounds": best[2], "bitmap_mean_error": best[0], "bitmap_difference_ratio": best[1]}


def article_persistence(database: Path) -> dict[str, Any]:
    preferences = setting(database, "preferences")
    article = preferences["article"]
    require(article["font_size"] == 20 and article["font"] == "song" and article["weight"] == 700 and article["gap"] == 2, "Article typography was not persisted")
    require(preferences["reader"]["font_size"] == 22 and preferences["reader"]["font"] == "song", "Editing article typography changed book typography")
    require_setting(database, "article.star:demo-article-0", True)
    return {"article_preferences": article, "local_favorite": True}


def clipboard_persistence(database: Path) -> dict[str, Any]:
    notes = rows(database, "SELECT * FROM annotations WHERE book_id='demo-0'")
    require(len(notes) == 1, "The actual Copy/Paste UI flow did not save exactly one thought")
    note = notes[0]
    require(note["chapter_index"] == 0, "The clipboard flow selected a different fixture chapter")
    body = scalar(database, "SELECT body FROM chapters WHERE book_id='demo-0' AND chapter_index=?", (note["chapter_index"],))
    require(body[note["start_offset"]:note["end_offset"]] == note["quote"], "Clipboard source no longer matches the selected chapter range")
    quote_text = note["quote"].strip()
    require(bool(quote_text) and note["note"] == quote_text, "The saved thought does not contain the actual copied passage")
    require(note["quote"] == CLIPBOARD_QUOTE and note["note"] == CLIPBOARD_QUOTE, "The pointer drag did not copy and save the exact four-glyph fixture passage")
    expected_start = body.index(CLIPBOARD_QUOTE)
    require((note["start_offset"], note["end_offset"]) == (expected_start, expected_start + len(CLIPBOARD_QUOTE)), "The clipboard range does not match the fixed four-glyph source anchor")
    require(scalar(database, "SELECT count(*) FROM query_history") == 0, "Clipboard thought text leaked into ordinary search history")
    return {"copied_quote": quote_text, "saved_thought": note["note"], "actual_pointer_copy_and_paste": True}


def settings_persistence(database: Path) -> dict[str, Any]:
    preferences = setting(database, "preferences")
    require(preferences["popular"] is False, "The enabled note toggle was not saved")
    require(preferences["edge_region"] == 25, "The edge-region stepper did not add its defined five-percent step")
    require(preferences["popup_height"] == 66, "The popup-height stepper did not add its defined four-percent step")
    require(preferences["download_article_images"] is True, "The article-image preference was not saved")
    require(preferences["download_concurrency"] == 9, "The chapter-download stepper did not retain its default-eight adjustment")
    require(preferences["report_time"] is True, "An unavailable remote reporting toggle accepted an offline mutation")
    return {key: preferences[key] for key in ("popular", "edge_region", "popup_height", "download_article_images", "download_concurrency", "report_time")}


def imported_book(database: Path) -> dict[str, Any]:
    books = rows(database, "SELECT * FROM books WHERE title=? AND on_shelf=1", (IMPORT_TITLE,))
    require(len(books) == 1, "The imported file did not become exactly one shelf record")
    chapters = rows(database, "SELECT body FROM chapters WHERE book_id=? ORDER BY chapter_index", (books[0]["id"],))
    require(len(chapters) == 2, "The two real TXT chapter headings were not imported")
    body = "\n".join(chapter["body"] for chapter in chapters)
    require("This local text was created by the UI verification harness." in body, "Imported chapter text was replaced with a placeholder")
    require("A second chapter makes the import and catalog checks independent of a cover." in body, "The second imported chapter body was lost")
    require(books[0]["cached_chapters"] == 2, "Imported cache metadata is inconsistent")
    return {"imported_id": books[0]["id"], "chapters": len(chapters)}


def pixel_difference(first: Path, second: Path, *, crop: tuple[int, int, int, int] | None = None, threshold: int = 16) -> float:
    with Image.open(first) as first_source, Image.open(second) as second_source:
        require(first_source.size == second_source.size, "Compared screenshots have different geometries")
        left, right = first_source.convert("RGB"), second_source.convert("RGB")
        if crop is not None:
            left, right = left.crop(crop), right.crop(crop)
        channels = ImageChops.difference(left, right).split()
        difference = ImageChops.lighter(ImageChops.lighter(channels[0], channels[1]), channels[2])
        histogram = difference.histogram()
        return sum(histogram[threshold + 1:]) / (difference.width * difference.height)


def changed(first: Capture, second: Capture, minimum: float) -> dict[str, Any]:
    difference = pixel_difference(first.path, second.path)
    require(difference > minimum, f"The expected interface state did not render a material change ({difference:.4%})")
    return {"difference_ratio": difference, "required_minimum": minimum}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output-dir", "--outputdir", dest="output_dir", type=Path, default=Path("artifacts/screenshots"))
    parser.add_argument("--data-dir", type=Path, help="New, empty root reserved for fixture-only metadata")
    parser.add_argument("--workers", type=int, choices=(1, 2, 3), default=2)
    parser.add_argument("--only", help="Comma-separated case group prefixes, for example snapshots or flows/reader")
    parser.add_argument("--baseline-dir", type=Path, help="Optional existing screenshot root with matching case paths")
    parser.add_argument("--max-diff-ratio", type=float, default=0.01)
    arguments = parser.parse_args()
    if os.name != "posix":
        parser.error("Run this harness on the designated remote Linux environment through ssh test-env")
    if not 0 <= arguments.max_diff_ratio <= 1:
        parser.error("--max-diff-ratio must be between zero and one")
    try:
        return Verifier(arguments).run()
    except Exception as error:
        print(f"UI verification setup failed: {redact(str(error))}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
