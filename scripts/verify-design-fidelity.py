#!/usr/bin/env python3
"""Verify design-review fixes on the designated remote Linux desktop worker.

Run only through ssh test-env. Every application process uses a disposable demo
database with networking disabled. Named external fixtures expose view-only QR,
verification-code and populated-review states; they are not authentication paths.
Reference PNGs are displayed at exactly half their original dimensions. Paired
images document matched UI states and are not whole-screen golden acceptance.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import sqlite3
import sys
from typing import Any, Callable, Iterable

from PIL import Image, ImageChops, ImageDraw


_spec = importlib.util.spec_from_file_location("weread_ui_verifier", Path(__file__).with_name("verify-ui.py"))
assert _spec is not None and _spec.loader is not None
ui = importlib.util.module_from_spec(_spec)
sys.modules[_spec.name] = ui
_spec.loader.exec_module(ui)
action, require = ui.action, ui.require


def grayscale(path: Path) -> Image.Image:
    with Image.open(path) as source:
        return source.convert("L")


def runs(values: Iterable[bool], minimum: int = 1) -> list[tuple[int, int]]:
    result: list[tuple[int, int]] = []
    start: int | None = None
    values = list(values)
    for index, value in enumerate([*values, False]):
        if value and start is None:
            start = index
        elif not value and start is not None:
            if index - start >= minimum:
                result.append((start, index))
            start = None
    return result


def horizontal_runs(image: Image.Image, y: int, x0: int, x1: int, *, tone: int = 100, minimum: int = 1) -> list[tuple[int, int]]:
    row = image.crop((x0, y, x1, y + 1)).tobytes()
    return [(x0 + start, x0 + end) for start, end in runs((value <= tone for value in row), minimum)]


def ink_ratio(image: Image.Image, box: tuple[int, int, int, int], *, maximum: int = 100) -> float:
    area = image.crop(box)
    histogram = area.histogram()
    return sum(histogram[:maximum + 1]) / (area.width * area.height)


def prepared_cover(source: Image.Image, target: tuple[int, int]) -> tuple[Image.Image, dict[str, Any]]:
    """Independent reference for the ordinary, non-dithered Article picture."""
    source = source.convert("L")
    width, height = target
    original_width, original_height = source.size
    if original_width * height >= original_height * width:
        crop_width = max(1, min(original_width, (original_height * width + height // 2) // height))
        crop_height = original_height
    else:
        crop_width = original_width
        crop_height = max(1, min(original_height, (original_width * height + width // 2) // width))
    x, y = (original_width - crop_width) // 2, (original_height - crop_height) // 2
    gray = source.crop((x, y, x + crop_width, y + crop_height)).resize(target, Image.Resampling.BILINEAR)
    raster = gray.point([min(15, (value + 8) // 17) * 17 for value in range(256)])
    return raster, {"source_rect": [x, y, crop_width, crop_height], "sampling": "center crop, triangle filter, standard Gray4 quantization"}


def matching_capture(first: ui.Capture, second: ui.Capture, box: tuple[int, int, int, int], *, maximum: float = 0.001) -> dict[str, Any]:
    difference = ui.pixel_difference(first.path, second.path, crop=box, threshold=8)
    require(difference <= maximum, f"The pointer state differs from its independently requested reference by {difference:.4%}")
    return {"crop": list(box), "difference_ratio": difference, "maximum_difference_ratio": maximum}


def locate_primary(image: Image.Image) -> tuple[int, int, int, int]:
    """Find the filled Scribe read action in the independent left pane."""
    x0, x1, y0, y1 = 42, 94, 430, 810
    # Sample the filled strip left of the centered label, not the white glyphs.
    rows = [ink_ratio(image, (x0, y, x1, y + 1), maximum=64) > 0.95 for y in range(y0, y1)]
    candidates = [(y0 + start, y0 + end) for start, end in runs(rows, 40)]
    require(len(candidates) == 1, f"Expected one filled read-button band, found {candidates}")
    top, bottom = candidates[0]
    require(bottom - top == 48, f"The actual primary button is {bottom - top}px high, expected the complete 48px target")
    edges = horizontal_runs(image, top + 8, 20, 310, tone=64, minimum=180)
    require(len(edges) == 1, f"Could not identify the actual primary button sides: {edges}")
    left, right = edges[0]
    require(30 <= left < 50 and 265 <= right <= 312, "The primary-button bounds escaped the summary pane")
    return left, top, right, bottom


def pager_geometry(capture: ui.Capture, right: int, center: int) -> dict[str, Any]:
    image = grayscale(capture.path)
    left = right - 152
    boxes = ((left, left + 48), (left + 48, right - 48), (right - 48, right))
    scores = [ink_ratio(image, (a + 8, center - 15, b - 8, center + 15), maximum=170) for a, b in boxes]
    require(all(score > 0.01 for score in scores), f"Expected visible previous, page label and next at the header's right edge: {scores}")
    return {"pager_bounds": [left, center - 24, right, center + 24], "segment_ink_ratios": scores}


def setting_write(database: Path, key: str, value: Any) -> None:
    require(database.is_file(), "The isolated fixture database does not exist")
    with sqlite3.connect(database) as connection:
        connection.execute("INSERT OR REPLACE INTO settings(key,value) VALUES(?,?)", (key, json.dumps(value, ensure_ascii=False)))


def annotation_history(database: Path) -> dict[str, Any]:
    with sqlite3.connect(database) as connection:
        annotations = connection.execute(
            "SELECT rowid,id,book_id,chapter_index,start_offset,end_offset,quote,note,created_at "
            "FROM annotations WHERE book_id='demo-0' ORDER BY id",
        ).fetchall()
        bookmarks = connection.execute(
            "SELECT id,book_id,chapter_index,char_offset,label,created_at "
            "FROM bookmarks WHERE book_id='demo-0' ORDER BY id",
        ).fetchall()
        queries = connection.execute("SELECT query,sequence FROM query_history ORDER BY query").fetchall()
    return {"annotations": annotations, "bookmarks": bookmarks, "query_history": queries,
        "underlines": ui.setting(database, "underlines:demo-0:demo-chapter-0"),
        "reviews": ui.setting(database, "thoughts:demo-0:demo-chapter-0:remote:0")}


def fixture_annotations(database: Path) -> dict[str, Any]:
    body = ui.scalar(database, "SELECT body FROM chapters WHERE book_id='demo-0' AND chapter_index=0")
    quote = body[:22]
    note_body = "每一次重读都会看到新的细节。"
    timestamp = 1_793_664_000
    review = {"id": "design-review", "author": "离线示例读者", "text": "文字的分量来自读者耐心的停留。",
        "likes": 12, "published_at": timestamp, "recommended": True}
    with sqlite3.connect(database) as connection:
        connection.execute(
            "INSERT OR REPLACE INTO annotations VALUES(?,?,?,?,?,?,?,?)",
            ("design-note", "demo-0", 0, 0, len(quote), quote, note_body, timestamp),
        )
        connection.execute(
            "INSERT OR REPLACE INTO bookmarks VALUES(?,?,?,?,?,?)",
            ("design-bookmark", "demo-0", 0, 0, "第一章", timestamp),
        )
    setting_write(database, "underlines:demo-0:demo-chapter-0", [{"range": "0-22", "text": quote, "count": 12, "thoughts": 2}])
    setting_write(database, "thoughts:demo-0:demo-chapter-0:remote:0", [review])
    return {"quote": quote, "note_id": "design-note", "note_body": note_body,
        "note_rowid": ui.scalar(database, "SELECT rowid FROM annotations WHERE id='design-note'"),
        "review_id": review["id"], "review_body": review["text"], "bookmark_id": "design-bookmark",
        "history": annotation_history(database)}


def fixture_library_pages(database: Path, mode: str) -> None:
    # Scribe's fifteen seeded covers fit exactly one page. Give every gesture
    # check at least three distinguishable pages so a no-op cannot pass.
    with sqlite3.connect(database) as connection:
        for index in range(45):
            connection.execute(
                "INSERT OR REPLACE INTO books(id,title,author,category,intro,cover_url,source,progress,total_chapters,cached_chapters,updated,finished,on_shelf) "
                "VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",
                (f"design-page-{index}", f"Page fixture {index + 1:02d}", "Offline fixture", "Fixture", "Isolated gesture verification data.", "", "demo", 0, 0, 0, 0, 0, 1),
            )
    preferences = ui.setting(database, "preferences")
    preferences["view_mode"] = mode
    setting_write(database, "preferences", preferences)


def fixture_uncached_chapters(database: Path) -> None:
    with sqlite3.connect(database) as connection:
        for index in range(3, 25):
            connection.execute(
                "INSERT OR REPLACE INTO chapters(book_id,chapter_index,source_id,title,body) VALUES(?,?,?,?,NULL)",
                ("demo-0", index, f"design-chapter-{index}", f"Uncached chapter {index + 1}"),
            )
        connection.execute("UPDATE books SET total_chapters=25 WHERE id='demo-0'")


def fixture_article(database: Path, *, cached: bool) -> Path | None:
    """Prepare a real local cache object, not an image-shaped UI placeholder."""
    url = "https://example.invalid/design-illustration.png"
    image_id = "image-" + hashlib.sha256(url.encode()).hexdigest()[:20]
    html = (
        "<blockquote>天朗气清，惠风和畅。仰观宇宙之大，俯察品类之盛。</blockquote>"
        "<p>这是一段独立的离线排版样本，用于检查完整行的两端对齐和引用字体。</p>"
        f'<img src="{url}" alt="离线几何图像样本" data-size="8192">'
    )
    setting_write(database, "article.html:demo-article-0", html)
    setting_write(database, "article.page:demo-article-0", 0)
    articles = ui.setting(database, "mp.articles")
    articles["demo-publication"][0]["original_url"] = "https://example.invalid/design-article"
    setting_write(database, "mp.articles", articles)
    if not cached:
        return None
    cache = database.parent / "covers"
    payload = Image.new("L", (1000, 400), 255)
    draw = ImageDraw.Draw(payload)
    draw.rectangle((0, 0, 999, 399), outline=0, width=8)
    draw.rectangle((45, 50, 370, 340), fill=170)
    draw.rectangle((420, 50, 940, 145), fill=221)
    draw.rectangle((420, 200, 940, 345), fill=68)
    temporary = cache / "design-fixture-payload.png"
    payload.save(temporary)
    data = temporary.read_bytes()
    # Use the current runtime cache namespace and length-delimited identity.
    fields = (b"demo-article-0", image_id.encode(), url.encode())
    identity = b"".join(len(field).to_bytes(8, "big") + field for field in fields)
    key = f"article-image:gray8-v2:{hashlib.sha256(identity).hexdigest()}"
    filename = f"{hashlib.sha256(key.encode()).hexdigest()}-{hashlib.sha256(data).hexdigest()}.png"
    destination = cache / filename
    temporary.rename(destination)
    with sqlite3.connect(cache / "cache-index.sqlite3") as connection:
        connection.execute("INSERT OR REPLACE INTO cache_entries VALUES(?,?,?,?,?)", (key, filename, len(data), 1, 0))
    return destination


class DesignVerifier(ui.Verifier):
    def __init__(self, arguments: argparse.Namespace) -> None:
        super().__init__(arguments)
        self.pairs: list[dict[str, Any]] = []

    def observe(self, name: str, *, fixture: str | None = None, **arguments: Any) -> ui.Capture:
        require(self.arguments.state_harness is not None, "A separately built offline state harness is required for this group")
        runner_arguments = ["--source-dir", str(self.repository)]
        if fixture is not None:
            runner_arguments.extend(["--fixture", fixture])
        return self.run_case(name, runner=self.arguments.state_harness, runner_arguments=runner_arguments, **arguments)

    def pair(self, capture: ui.Capture, target: str, *, state: str, exclusions: list[str] | None = None) -> None:
        if self.arguments.reference_dir is None:
            return
        reference = self.arguments.reference_dir / target
        require(reference.is_file(), f"The requested design reference is missing: {reference}")
        with Image.open(reference) as source, Image.open(capture.path) as actual_source:
            require(source.width % 2 == 0 and source.height % 2 == 0, "The design reference is not exactly divisible by its 2px/dp scale")
            expected = source.convert("RGB").resize((source.width // 2, source.height // 2), Image.Resampling.LANCZOS)
            actual = actual_source.convert("RGB")
            require(expected.size == actual.size, f"Reference geometry {expected.size} differs from actual {actual.size}")
            sheet = Image.new("RGB", (expected.width * 2 + 24, expected.height + 28), "white")
            sheet.paste(expected, (0, 28))
            sheet.paste(actual, (expected.width + 24, 28))
            draw = ImageDraw.Draw(sheet)
            draw.text((8, 8), "DESIGN v0.2 (0.5x)", fill="black")
            draw.text((expected.width + 32, 8), "IMPLEMENTATION (1x)", fill="black")
            output = self.output / "comparisons" / f"{capture.path.stem}-{target}"
            output.parent.mkdir(parents=True, exist_ok=True)
            sheet.save(output)
        record = {
            "state": state, "reference": str(reference), "actual": str(capture.path), "comparison": str(output),
            "reference_scale": 0.5, "implementation_scale": 1,
            "excluded_from_pixel_acceptance": exclusions or ["Fixture text, counts, dates, source identity and QR payload"],
            "acceptance": "Targeted structural assertions are reported separately; this pair is review evidence, not a whole-screen pixel pass.",
        }
        with self.lock:
            self.pairs.append(record)

    def catalog(self, profile: str) -> None:
        prefix = f"design/catalog/{profile}"
        if profile == "scribe":
            self.execute_group(f"{prefix}/primary", lambda: self.catalog_primary(profile))
        self.execute_group(f"{prefix}/detail", lambda: self.catalog_detail(profile))
        for mode in ("cover", "list"):
            self.execute_group(f"{prefix}/{mode}", lambda selected_mode=mode: self.catalog_mode(profile, selected_mode))

    def catalog_mode(self, profile: str, mode: str) -> None:
        width, height = ui.PROFILES[profile]
        prefix = f"design/catalog/{profile}"
        name = f"{prefix}/{mode}"
        seed = self.run_case(f"{name}/seed", profile=profile, data_key=name)
        fixture_library_pages(seed.database, mode)
        before = self.run_case(f"{name}/before", profile=profile, data_key=name)
        expected = self.run_case(f"{name}/expected-page-two", profile=profile, data_key=name, actions=[action(400, "library", "page", "1")])
        pressed = self.run_case(f"{name}/header-pager-next", profile=profile, data_key=name,
            actions=[action(400, "pointer", "tap", f"{width - 32},104")])
        self.assertion(f"{name}/header-pager-has-three-visible-segments", lambda: pager_geometry(before, width - 8, 104), before)
        self.assertion(f"{name}/header-next-pages-exactly-once", lambda: matching_capture(expected, pressed, (0, 80, width, height)), expected, pressed)
        drag = [
            action(400, "pointer", "down", f"{width - 100},300"),
            action(550, "pointer", "move", "100,300"),
            action(700, "pointer", "up", "100,300"),
        ]
        swiped = self.run_case(f"{name}/horizontal-swipe", profile=profile, data_key=name, actions=drag)
        self.assertion(f"{name}/swipe-pages-exactly-once", lambda a=expected, b=swiped: matching_capture(a, b, (0, 132, width, height)), expected, swiped)
        self.assertion(f"{name}/page-two-is-distinct", lambda a=before, b=swiped: ui.changed(a, b, 0.01), before, swiped)
        try:
            vertical = self.run_case(f"{name}/vertical-drag", profile=profile, data_key=name, actions=[
                action(400, "pointer", "down", "110,300"), action(550, "pointer", "move", "110,450"), action(700, "pointer", "up", "110,450")])
            self.assertion(f"{name}/vertical-drag-does-not-open-or-page", lambda a=before, b=vertical: matching_capture(a, b, (0, 132, width, height)), before, vertical)
        except ui.CaseFailure:
            pass
        self.run_case(f"{name}/short-tap-opens-detail", profile=profile,
            data_key=name, actions=[action(400, "pointer", "tap", "110,300")], expected_route="detail")

    def catalog_detail(self, profile: str) -> None:
        width, height = ui.PROFILES[profile]
        prefix = f"design/catalog/{profile}"
        intro = self.run_case(f"{prefix}/detail-intro-request", profile=profile, screen="detail", actions=[action(200, "detail", "tab", "intro")])
        selected_intro = self.run_case(f"{prefix}/detail-selected-intro", profile=profile, screen="detail", actions=[action(200, "detail", "select"), action(400, "detail", "tab", "intro")])
        self.assertion(f"{prefix}/selection-to-intro-clears-action-bar", lambda: matching_capture(intro, selected_intro, (0, 28, width, height)), intro, selected_intro)
        toc = self.run_case(f"{prefix}/detail-toc", profile=profile, screen="detail")
        cleared_toc = self.run_case(f"{prefix}/detail-selected-intro-toc", profile=profile, screen="detail", actions=[action(200, "detail", "select"), action(400, "detail", "tab", "intro"), action(600, "detail", "tab", "toc")])
        self.assertion(f"{prefix}/selection-mode-remains-cleared", lambda: matching_capture(toc, cleared_toc, (0, 28, width, height)), toc, cleared_toc)
        selected = self.run_case(f"{prefix}/detail-selected", profile=profile, screen="detail", actions=[action(200, "detail", "select")])
        self.assertion(f"{prefix}/selection-actions-are-in-page-header", lambda: self.selection_header(toc, selected, profile), toc, selected)
        stem = "SC" if profile == "scribe" else "PW"
        if profile != "scribe":
            self.pair(intro, f"{stem}-Detail--intro.png", state="Detail Introduction")
        self.pair(selected, f"{stem}-Detail--select.png", state="Detail chapter selection")
        selection_key = f"{prefix}/picked-fixture"
        selection_seed = self.run_case(f"{prefix}/picked-seed", profile=profile, screen="detail", data_key=selection_key)
        fixture_uncached_chapters(selection_seed.database)
        pick_actions = [action(200, "detail", "select"), action(400, "detail", "chapter", "design-chapter-3")]
        picked = self.observe(f"{prefix}/picked-one", profile=profile, screen="detail", data_key=selection_key, actions=pick_actions)
        self.assertion(f"{prefix}/fixture-really-selects-an-uncached-chapter", lambda: self.chapter_selection(picked, "toc", True, 1), picked)
        cleared = self.observe(f"{prefix}/picked-then-intro", profile=profile, screen="detail", data_key=selection_key, actions=[*pick_actions, action(600, "detail", "tab", "intro")])
        projected_tab = "toc" if profile == "scribe" else "intro"
        self.assertion(f"{prefix}/picked-count-and-mode-clear-on-intro-request", lambda: self.chapter_selection(cleared, projected_tab, False, 0), cleared)
        select_all = self.observe(f"{prefix}/picked-all-there-is-room-for", profile=profile, screen="detail", data_key=selection_key,
            actions=[action(200, "detail", "select"), action(400, "detail", "select-page")])
        uncached = (20 if profile == "scribe" else 8) - 3
        self.assertion(f"{prefix}/selection-does-not-reduce-toc-capacity", lambda: self.chapter_selection(select_all, "toc", True, uncached), select_all)
        if profile == "scribe":
            with sqlite3.connect(picked.database) as connection:
                short_intro, = connection.execute("SELECT intro FROM books WHERE id='demo-0'").fetchone()
                connection.execute("UPDATE books SET intro=? WHERE id='demo-0'", (short_intro * 7,))
            full = self.run_case(f"{prefix}/full-introduction", profile=profile, screen="detail", data_key=selection_key,
                actions=[action(300, "detail", "intro-open")])
            self.assertion(f"{prefix}/full-introduction-dialog-uses-design-anchor",
                lambda: self.introduction_dialog(full), full)

    def catalog_primary(self, profile: str) -> None:
        width, height = ui.PROFILES[profile]
        prefix = f"design/catalog/{profile}"
        toc = self.run_case(f"{prefix}/detail-primary-baseline", profile=profile, screen="detail")
        bounds = locate_primary(grayscale(toc.path))
        left, top, right, bottom = bounds
        self.assertion(f"{prefix}/read-button-visible-height", lambda: {"actual_button_bounds": list(bounds), "height": bottom - top}, toc)
        for label, x, y in (("center", (left + right) // 2, (top + bottom) // 2), ("lower-center", (left + right) // 2, bottom - 2), ("lower-left", left + 10, bottom - 2)):
            try:
                self.run_case(f"{prefix}/read-hit-{label}", profile=profile, screen="detail", actions=[action(300, "pointer", "tap", f"{x},{y}")], expected_route="reader")
            except ui.CaseFailure:
                pass
        for tab, x in (("toc", 430), ("reviews", 617), ("notes", 804)):
            expected = self.run_case(f"{prefix}/expected-tab-{tab}", profile=profile, screen="detail", actions=[action(250, "detail", "tab", tab)])
            tapped = self.run_case(f"{prefix}/touch-tab-{tab}", profile=profile, screen="detail", actions=[action(250, "pointer", "tap", f"{x},128")])
            self.assertion(f"{prefix}/tab-{tab}-opens-in-right-pane", lambda a=expected, b=tapped: matching_capture(a, b, (336, 104, width, height)), expected, tapped)

    @staticmethod
    def chapter_selection(capture: ui.Capture, tab: str, selecting: bool, count: int) -> dict[str, Any]:
        detail = capture.state["detail"]
        require(detail["tab"] == tab and detail["selecting"] is selecting and detail["selected_count"] == count, "The chapter selection state does not match the real uncached selection transition")
        return {"tab": detail["tab"], "selecting": detail["selecting"], "selected_count": detail["selected_count"]}

    @staticmethod
    def selection_header(normal: ui.Capture, selecting: ui.Capture, profile: str) -> dict[str, Any]:
        width, height = ui.PROFILES[profile]
        image = grayscale(selecting.path)
        box = (8, 28, width - 8, 79)
        change = ui.pixel_difference(normal.path, selecting.path, crop=box, threshold=8)
        require(change > 0.015, "The normal title/back/more header did not change to a selection action bar")
        lines = [y for y in range(height - 90, height) if ink_ratio(image, (4, y, width - 4, y + 1), maximum=64) > 0.97]
        require(not lines, f"A bottom selection action bar remains visible: {lines}")
        require(ink_ratio(image, (width // 2 - 50, 40, width // 2 + 50, 70)) > 0.035, "The selection count is missing from the center of the header")
        require(ink_ratio(image, (width - 145, 30, width - 12, 77), maximum=190) > 0.02, "The offline download action is not visibly outlined at the right of the header")
        return {"header_bounds": list(box), "header_difference_ratio": change, "bottom_action_bar": False}

    @staticmethod
    def introduction_dialog(capture: ui.Capture) -> dict[str, Any]:
        image = grayscale(capture.path)
        left, top, right = (image.width - 580) // 2, 160, (image.width + 580) // 2
        require(ink_ratio(image, (left + 8, top, right - 8, top + 2), maximum=64) > 0.99,
                "The full introduction is not framed at the Scribe dialog anchor")
        return {"dialog_top": top, "dialog_left": left, "dialog_width": right - left}

    def auth(self, profile: str) -> None:
        width, height = ui.PROFILES[profile]
        prefix = f"design/auth/{profile}"
        key = f"{prefix}/history"
        seed = [action(200, "search", "scope", "shelf"), action(350, "search", "edit-query", "古文"), action(500, "search", "key", "submit"), action(650, "search", "clear")]
        empty = self.run_case(f"{prefix}/recent", profile=profile, screen="search", data_key=key, actions=seed)
        composing = self.run_case(f"{prefix}/composing", profile=profile, screen="search", data_key=key, actions=[action(200, "search", "scope", "shelf"), action(350, "search", "key", "s"), action(500, "search", "key", "h"), action(650, "search", "key", "i")])
        self.assertion(f"{prefix}/uncommitted-composition-retains-recent-view", lambda: matching_capture(empty, composing, (0, 136, width, height - 300), maximum=0), empty, composing)
        self.assertion(f"{prefix}/composition-is-inline-and-underlined", lambda: self.inline_composition(empty, composing, profile), empty, composing)
        self.assertion(f"{prefix}/composition-is-not-submitted", lambda: {"query_history_count": ui.require_query(composing.database, "古文"), "uncommitted_pinyin_rows": self.no_pinyin_history(composing.database)}, composing)
        self.assertion(f"{prefix}/keyboard-divider-is-at-top", lambda: self.keyboard_divider(composing, profile), composing)
        self.pair(composing, f"{'SC' if profile == 'scribe' else 'PW'}-Search--pinyin.png", state="Search composing pinyin with recent history")
        committed = self.observe(f"{prefix}/composition-after-committed-query", profile=profile, screen="search", actions=[
            action(200, "search", "scope", "shelf"), action(350, "search", "edit-query", "古文"),
            action(500, "search", "key", "s"), action(650, "search", "key", "h"), action(800, "search", "key", "i"),
        ])
        self.assertion(f"{prefix}/composition-follows-committed-text", lambda: self.committed_composition(committed, profile), committed)
        with sqlite3.connect(composing.database) as connection:
            connection.execute("DELETE FROM query_history")
            for index, term in enumerate(("古文", "稍长的搜索词", "短词", "离线搜索布局样本" * 5), 1):
                connection.execute("INSERT INTO query_history VALUES(?,?)", (term, index))
        wrapped = self.observe(f"{prefix}/wrapped-history", profile=profile, screen="search", data_key=key)
        self.assertion(f"{prefix}/history-uses-content-width-wrapping-chips", lambda: self.history_chips(wrapped, profile), wrapped)
        fixture_library_pages(wrapped.database, "cover")
        query = [action(200, "search", "scope", "shelf"), action(350, "search", "edit-query", "Page fixture")]
        typing = self.observe(f"{prefix}/results-with-keyboard", profile=profile, screen="search", data_key=key, actions=query)
        submitted = self.observe(f"{prefix}/results-without-keyboard", profile=profile, screen="search", data_key=key,
            actions=[*query, action(550, "search", "key", "submit")])
        right = width - (40 if profile == "scribe" else 8)
        for keyboard, capture in ((True, typing), (False, submitted)):
            rows = (11 if keyboard else 15) if profile == "scribe" else (5 if keyboard else 9)
            self.assertion(f"{prefix}/{'with' if keyboard else 'without'}-keyboard-fits-{rows}-results",
                lambda c=capture, k=keyboard, n=rows: self.search_results(c, k, n, right), capture)
        empty_result = self.observe(f"{prefix}/submitted-no-matches", profile=profile, screen="search", data_key=key, actions=[
            action(200, "search", "scope", "shelf"), action(350, "search", "edit-query", "No matching fixture"),
            action(550, "search", "key", "submit"), action(750, "pointer", "tap", f"{right - 76},112"),
        ])
        self.assertion(f"{prefix}/empty-result-keeps-single-page-header-pager", lambda: self.empty_search_pager(empty_result, right), empty_result)

    @staticmethod
    def committed_composition(capture: ui.Capture, profile: str) -> dict[str, Any]:
        search = capture.state["search"]
        require(search["query"] == "古文" and search["composition"] == "shi", "Committed text and uncommitted pinyin were merged or lost")
        require(search["result_count"] > 0, "Composition replaced the committed query's local result set")
        field_start = 126 if profile == "scribe" else 66
        left = round(field_start + search["query_width"])
        right = round(left + search["composition_width"])
        image = grayscale(capture.path)
        require(right - left >= 18, "The measured pinyin width is not large enough for an independent underline check")
        border_rows = [y for y in range(28, 40) if ink_ratio(image, (field_start + 10, y, image.width - 150, y + 1), maximum=64) > 0.99]
        require(bool(border_rows), "The search input has no identifiable top border")
        underline_y = min(border_rows) + (48 - 28) // 2 + 25
        require(ink_ratio(image, (left + 2, underline_y, right - 2, underline_y + 1), maximum=64) > 0.99, "The composition underline does not directly follow the committed query")
        return {"query": search["query"], "composition": search["composition"], "underline_bounds": [left, underline_y, right, underline_y + 1], "committed_result_count": search["result_count"]}

    @staticmethod
    def search_results(capture: ui.Capture, keyboard: bool, rows: int, right: int) -> dict[str, Any]:
        search = capture.state["search"]
        require(search["keyboard_visible"] is keyboard and search["results_visible"], "Results or keyboard do not match the requested state")
        require(search["page"] == 1 and search["page_count"] == (45 + rows - 1) // rows and search["result_count"] == rows,
                f"Search paginates a different number of result rows: {search}")
        return {**pager_geometry(capture, right, 112), "keyboard_open": keyboard, "rows": rows, "pages": search["page_count"]}

    @staticmethod
    def empty_search_pager(capture: ui.Capture, right: int) -> dict[str, Any]:
        search = capture.state["search"]
        require(search["results_visible"] and search["no_results"] and search["page"] == search["page_count"] == 1,
                "A submitted empty search lost its visible single-page header")
        require(not capture.state["shell"]["dialog_kind"], "The single-page label wrongly opened a jump dialog")
        return {**pager_geometry(capture, right, 112), "page": 1, "page_count": 1, "label_interaction": "inert"}

    @staticmethod
    def history_chips(capture: ui.Capture, profile: str) -> dict[str, Any]:
        search = capture.state["search"]
        chips = search["recent_chips"]
        require(len(chips) == 4 and search["recent_rows"] >= 2, "The long recent-query fixture did not wrap into multiple rows")
        require(len({chip["width"] for chip in chips}) > 1, "Recent queries are still equal-width tiles")
        margin = 32 if profile == "scribe" else 20
        available = ui.PROFILES[profile][0] - margin * 2
        for chip in chips:
            require(chip["width"] >= 48 and chip["x"] >= 0 and chip["x"] + chip["width"] <= available + 0.001, "A recent-query target is too small or escapes its content row")
        first = chips[0]
        left, right = round(margin + first["x"]), round(margin + first["x"] + first["width"])
        top = 219 + first["row"] * 56
        image = grayscale(capture.path)
        require(any(ink_ratio(image, (left + 2, y, right - 2, y + 1), maximum=190) > 0.95 for y in (top, top + 1)), "The recent-query chip lacks its 36px square outline")
        return {"rows": search["recent_rows"], "chip_geometry": chips, "first_outline_top": top, "touch_height": 48, "row_gap": 8}

    @staticmethod
    def no_pinyin_history(database: Path) -> int:
        count = ui.scalar(database, "SELECT count(*) FROM query_history WHERE query='shi'")
        require(count == 0, "Uncommitted pinyin was written to search history")
        return count

    @staticmethod
    def inline_composition(empty: ui.Capture, composing: ui.Capture, profile: str) -> dict[str, Any]:
        width, _ = ui.PROFILES[profile]
        left = 32 if profile == "scribe" else 20
        region = (left + (64 if profile == "scribe" else 0), 34, width - 130, 82)
        a, b = grayscale(empty.path).crop(region), grayscale(composing.path).crop(region)
        # Ignore the removed gray placeholder and retain newly drawn black ink.
        # The old detached composition is then visible as a far-right change.
        old_ink = a.point(lambda value: 255 if value <= 80 else 0)
        new_ink = b.point(lambda value: 255 if value <= 80 else 0)
        difference = ImageChops.subtract(new_ink, old_ink)
        bounds = difference.getbbox()
        require(bounds is not None, "Typing pinyin did not change the search input")
        x0, y0, x1, y1 = bounds
        require(x0 < 70 and x1 < 160, f"Composition moved away from the inline input start: {bounds}")
        image = grayscale(composing.path)
        lines = []
        for y in range(region[1] + y0 + 8, min(region[1] + y1 + 3, 82)):
            lines.extend((start, y, end - start) for start, end in horizontal_runs(image, y, region[0], region[0] + 170, tone=120, minimum=18))
        require(bool(lines), "The inline pinyin has no visible continuous underline")
        return {"input_change_bounds": [region[0] + x0, region[1] + y0, region[0] + x1, region[1] + y1], "underline_runs": lines}

    @staticmethod
    def keyboard_divider(capture: ui.Capture, profile: str) -> dict[str, Any]:
        width, height = ui.PROFILES[profile]
        image = grayscale(capture.path)
        top = height - 300
        actual = [y for y in range(top - 3, top + 4) if ink_ratio(image, (0, y, width, y + 1), maximum=64) > 0.98]
        require(actual == [top, top + 1], f"The keyboard divider is absent from the top edge: {actual}")
        return {"keyboard_top": top, "divider_rows": actual}

    def controls(self, profile: str) -> None:
        prefix = f"design/controls/{profile}"
        for key, subpage, options, preference_path in (
            ("image_dithering", "display", ("off", "software", "hardware"), ("image_dithering",)),
            ("tap_mode", "notes", ("default", "mirrored", "swipe"), ("reader", "tap_mode")),
            ("popup_position", "notes", ("side", "center", "bottom") if profile == "scribe" else ("center", "bottom"), ("popup_position",)),
        ):
            for value in options:
                capture = self.run_case(f"{prefix}/{key}-{value}", profile=profile, screen="settings", actions=[
                    action(200, "auxiliary", "settings.open", subpage), action(400, "auxiliary", "settings.choice", f"{key}:{value}"), action(600, "auxiliary", "settings.choice", f"{key}:{value}")])
                def persisted(current: ui.Capture = capture, path: tuple[str, ...] = preference_path, expected: str = value) -> dict[str, Any]:
                    preferences = ui.setting(current.database, "preferences")
                    result = preferences
                    for part in path:
                        result = result[part]
                    require(result == expected, f"Direct choice did not persist its absolute value: {result!r} != {expected!r}")
                    return {"preference_path": list(path), "absolute_value": result, "repeated_choice_is_idempotent": True}
                self.assertion(f"{prefix}/{key}-{value}-absolute-and-idempotent", persisted, capture)
        width, _ = ui.PROFILES[profile]
        margin = 32 if profile == "scribe" else 20
        # Settings rows start below the status bar, the account header and the
        # tab bar plus an 8-pixel gap. A choice row is one line: its 48-pixel
        # segmented control is centred in the row and right-aligned at the
        # content edge, three choices 120 (Scribe) or 100 (Paperwhite) wide.
        rows_top, row_height = (204, 64) if profile == "scribe" else (184, 56)
        three = 120 if profile == "scribe" else 100

        def segments(row: int, count: int, option_width: int) -> tuple[int, int, int, int]:
            top = rows_top + row * row_height + (row_height - 48) // 2
            return (width - margin - count * option_width, top, width - margin, top + 48)

        dither = segments(0, 3, three)
        dither_x = dither[0] + three * 1.5
        pointer_dither = self.run_case(f"{prefix}/dither-pointer-software", profile=profile, screen="settings", actions=[
            action(200, "auxiliary", "settings.open", "display"),
            action(400, "pointer", "tap", f"{dither_x},{dither[1] + 24}"),
            action(600, "pointer", "tap", f"{dither_x},{dither[1] + 24}"),
        ])
        self.assertion(f"{prefix}/dither-pointer-selects-the-specified-segment", lambda: self.preference_value(pointer_dither, ("image_dithering",), "software"), pointer_dither)
        self.assertion(f"{prefix}/dither-segments-share-one-frame", lambda: self.segmented_frame(pointer_dither, dither, 3), pointer_dither)
        # Reading tab: tap zones first, then the note position.
        tap = segments(0, 3, three)
        tap_x = tap[0] + three * 1.5
        pointer_tap = self.run_case(f"{prefix}/tap-pointer-mirrored", profile=profile, screen="settings", actions=[
            action(200, "auxiliary", "settings.open", "notes"),
            action(400, "pointer", "tap", f"{tap_x},{tap[1] + 24}"),
            action(600, "pointer", "tap", f"{tap_x},{tap[1] + 24}"),
        ])
        self.assertion(f"{prefix}/tap-pointer-selects-middle-segment", lambda: self.preference_value(pointer_tap, ("reader", "tap_mode"), "mirrored"), pointer_tap)
        popup_width = 120 if profile == "scribe" else 96
        popup = segments(1, 3 if profile == "scribe" else 2, popup_width)
        popup_x = popup[2] - popup_width / 2
        pointer_popup = self.run_case(f"{prefix}/popup-pointer-bottom", profile=profile, screen="settings", actions=[
            action(200, "auxiliary", "settings.open", "notes"),
            action(400, "pointer", "tap", f"{popup_x},{popup[1] + 24}"),
            action(600, "pointer", "tap", f"{popup_x},{popup[1] + 24}"),
        ])
        self.assertion(f"{prefix}/popup-pointer-selects-last-segment", lambda: self.preference_value(pointer_popup, ("popup_position",), "bottom"), pointer_popup)
        for route, section, tool_action, maximum in (("reader", "reader", "tool", 38 if profile == "scribe" else 32), ("article", "article", "article.tool", 32 if profile == "scribe" else 26)):
            name = f"{prefix}/{route}-size"
            capture = self.run_case(name, profile=profile, screen=route, actions=[action(100, section, "menu" if route == "reader" else "article.menu", "true"), action(200, section, tool_action, "type"), action(400, section, "font-size-select" if route == "reader" else "article.font-size-select", str(maximum)), action(600, section, "font-size-select" if route == "reader" else "article.font-size-select", str(maximum))])
            self.assertion(f"{name}-absolute-and-idempotent", lambda c=capture, r=route, expected=maximum: self.font_size(c, r, expected), capture)
            self.pair(capture, f"{'SC' if profile == 'scribe' else 'PW'}-{'Reader' if route == 'reader' else 'Article'}--type.png", state=f"{route} direct typography selection", exclusions=["The chosen largest font differs from the reference default; compare scale structure only", "Fixture reading content"])

        for route, section, menu_action, tool_action, heights, scale_offset, largest in (
            ("reader", "reader", "menu", "tool", (384, 344), (92, 88), (38, 32)),
            ("article", "article", "article.menu", "article.tool", (216, 308), (76, 68), (32, 26)),
        ):
            index = 0 if profile == "scribe" else 1
            events = [action(100, section, menu_action, "true"), action(200, section, tool_action, "type")]
            menu = self.run_case(f"{prefix}/{route}-typography-surface", profile=profile, screen=route, actions=events)
            applied_height = heights[index]
            panel_top = self.panel_geometry(menu, profile, 5 if route == "reader" else 4, applied_height)["panel_top"]
            # The last scale point is 24px before the track end, separate from the large-A step button.
            x, y = width - margin - 78, panel_top + scale_offset[index]
            capture = self.run_case(f"{prefix}/{route}-size-pointer", profile=profile, screen=route,
                actions=[*events, action(400, "pointer", "tap", f"{x},{y}"), action(600, "pointer", "tap", f"{x},{y}")])
            self.assertion(f"{prefix}/{route}-size-pointer-selects-last-scale-point",
                lambda c=capture, r=route, expected=largest[index]: self.font_size(c, r, expected), capture)

    @staticmethod
    def preference_value(capture: ui.Capture, path: tuple[str, ...], expected: Any) -> dict[str, Any]:
        value = ui.setting(capture.database, "preferences")
        for key in path:
            value = value[key]
        require(value == expected, f"The actual touch target selected {value!r} instead of {expected!r}")
        return {"preference_path": list(path), "actual_value": value, "repeated_pointer_choice_is_idempotent": True}

    @staticmethod
    def segmented_frame(capture: ui.Capture, box: tuple[int, int, int, int], count: int) -> dict[str, Any]:
        image = grayscale(capture.path)
        left, top, right, bottom = box
        require(ink_ratio(image, (left + 8, top, right - 8, top + 2), maximum=64) > 0.99, "The segmented choices do not have one continuous top frame")
        require(ink_ratio(image, (left + 8, bottom - 2, right - 8, bottom), maximum=64) > 0.99, "The segmented choices do not have one continuous bottom frame")
        dividers = []
        for index in range(1, count):
            x = round(left + (right - left) * index / count)
            require(any(ink_ratio(image, (candidate, top + 5, candidate + 1, bottom - 5), maximum=64) > 0.99 for candidate in (x - 1, x, x + 1)), "A straight internal segment divider is missing")
            dividers.append(x)
        return {"frame_bounds": list(box), "shared_frame": True, "straight_divider_x": dividers}

    @staticmethod
    def font_size(capture: ui.Capture, route: str, expected: int) -> dict[str, Any]:
        preferences = ui.setting(capture.database, "reader.preferences:demo-0") if route == "reader" else ui.setting(capture.database, "preferences")["article"]
        require(preferences["font_size"] == expected, f"The direct {route} size choice did not save {expected}")
        return {"font_size": preferences["font_size"], "absolute_selection_repeated": True}

    def observed(self, profile: str) -> None:
        prefix = f"design/observed/{profile}"
        stem = "SC" if profile == "scribe" else "PW"
        for fixture, route, target in (("login-qr", "login", f"{stem}-Login.png"), ("login-code", "login", f"{stem}-Login--code.png"), ("detail-reviews", "detail", f"{stem}-Detail--reviews.png"), ("detail-notes", "detail", f"{stem}-Detail--notes.png"), ("article-qr", "article", f"{stem}-Article--qr.png")):
            if profile != "scribe" and fixture == "detail-notes":
                continue
            capture = self.observe(f"{prefix}/{fixture}", profile=profile, screen=route, fixture=fixture)
            self.assertion(f"{prefix}/{fixture}-is-explicit-offline-view-fixture", lambda c=capture, f=fixture: self.fixture_boundary(c, f), capture)
            if fixture == "article-qr":
                self.assertion(f"{prefix}/qr-card-is-centered-and-content-sized", lambda c=capture: self.qr_geometry(c, profile), capture)
            self.pair(capture, target, state=fixture)
        for fixture in ("login-loading", "login-expired", "login-code-empty"):
            capture = self.observe(f"{prefix}/{fixture}", profile=profile, screen="login", fixture=fixture)
            self.assertion(f"{prefix}/{fixture}-is-view-only", lambda c=capture, f=fixture: self.login_fixture(c, f), capture)
        for fixture in ("detail-download", "detail-downloading"):
            capture = self.observe(f"{prefix}/{fixture}", profile=profile, screen="detail", fixture=fixture)
            self.assertion(f"{prefix}/{fixture}-shows-only-its-decision-or-progress-copy",
                lambda c=capture, f=fixture: self.download_fixture(c, f), capture)
        library = self.observe(f"{prefix}/library-list", profile=profile, actions=[action(200, "library", "view")])
        self.assertion(f"{prefix}/cached-list-book-retains-download-icon", lambda: self.cached_list_icon(library, profile), library)
        self.pair(library, f"{stem}-Library--list.png", state="Library list with a cached first book")

    def foundation(self, profile: str) -> None:
        prefix = f"design/foundation/{profile}"
        width, _ = ui.PROFILES[profile]
        margin = 32 if profile == "scribe" else 20
        for route in ("reader", "article"):
            events = [action(200, "reader", "bookmark"), action(400, "reader", "menu", "true")] if route == "reader" else [action(200, "article", "article.star"), action(400, "article", "article.menu", "true")]
            capture = self.run_case(f"{prefix}/{route}-selected-header", profile=profile, screen=route, actions=events, exit_ms=3_000)
            self.assertion(f"{prefix}/{route}-header-icon-has-fixed-touch-width", lambda c=capture: self.header_icon_width(c), capture)
        # Offline fixtures have no web session, so the Sync tab's reporting switches are disabled.
        sync = self.run_case(f"{prefix}/disabled-sync", profile=profile, screen="settings", actions=[action(200, "auxiliary", "settings.open", "sync")])
        self.assertion(f"{prefix}/disabled-switch-outline-is-dashed", lambda: self.dashed_switch(sync, profile), sync)
        statistics = self.run_case(f"{prefix}/stats-segments", profile=profile, screen="stats")
        top = 92 if profile == "scribe" else 84
        right = margin + 380 if profile == "scribe" else width - margin
        self.assertion(f"{prefix}/stats-periods-share-one-segmented-frame", lambda: self.segmented_frame(statistics, (margin, top, right, top + 48), 4), statistics)
        self.pair(statistics, f"{'SC' if profile == 'scribe' else 'PW'}-Stats.png", state="Statistics week controls", exclusions=["Date-dependent reading data and source notices"])
        for route, state, offset, actions in (
            ("detail", "no-bottom-chrome", 24, [action(300, "detail", "download")]),
            ("search", "keyboard-open", 316, [action(300, "search", "add", "demo-0")]),
            ("search", "keyboard-closed", 24, [action(150, "search", "key", "submit"), action(300, "search", "add", "demo-0")]),
            ("reader", "menu-closed", 64 if profile == "scribe" else 56, [action(300, "reader", "sync")]),
            ("article", "menu-closed", 72 if profile == "scribe" else 56, [action(300, "article", "article.star")]),
        ):
            # The desktop fixture requests a local toast through the normal action handler.
            # This checks anchoring, not the availability of an offline download or store operation.
            toast = self.run_case(f"{prefix}/{route}-{state}-toast", profile=profile, screen=route, actions=actions)
            self.assertion(f"{prefix}/{route}-{state}-toast-anchor", lambda c=toast, y=offset: ui.bottom_toast(c, y), toast)

    @staticmethod
    def header_icon_width(capture: ui.Capture) -> dict[str, Any]:
        image = grayscale(capture.path)
        width = image.width
        # Inspect the filled background's continuous rows. Counting the whole
        # column would incorrectly split the box at the inverted white glyph.
        bands = [(left, y, right) for y in range(28, 78) for left, right in horizontal_runs(image, y, width - 260, width - 8, tone=64, minimum=40)]
        require(bool(bands), "The selected header icon has no filled touch background")
        widest = max(bands, key=lambda band: band[2] - band[0])
        require(widest[2] - widest[0] == 48, f"The selected header icon does not occupy one compact 48px target: {widest}")
        return {"selected_icon_x": [widest[0], widest[2]], "sample_row": widest[1], "touch_width": widest[2] - widest[0]}

    @staticmethod
    def dashed_switch(capture: ui.Capture, profile: str) -> dict[str, Any]:
        image = grayscale(capture.path)
        margin = 32 if profile == "scribe" else 20
        rows_top, row_height = (204, 64) if profile == "scribe" else (184, 56)
        # Switches are 48 x 28, right-aligned at the content edge and centred in their row.
        left = image.width - margin - 48
        for row in range(6):
            top = rows_top + row * row_height + (row_height - 28) // 2
            values = list(image.crop((left + 7, top, left + 41, top + 1)).tobytes())
            dash_runs = runs((value <= 190 for value in values), 1)
            gap_runs = runs((value >= 240 for value in values), 1)
            if len(dash_runs) >= 3 and len(gap_runs) >= 2 and min(values) >= 160:
                return {"row": row, "sample_line": [left + 7, top, left + 41, top + 1], "dash_runs": dash_runs, "gap_runs": gap_runs, "darkest_tone": min(values)}
        require(False, "No disabled switch on the Sync tab shows a gray dashed outline")
        return {}

    @staticmethod
    def fixture_boundary(capture: ui.Capture, fixture: str) -> dict[str, Any]:
        require(capture.state.get("fixture") == fixture, "The fixture runner did not identify its explicit display overlay")
        require(capture.state.get("shell", {}).get("online") is False, "The view fixture is not offline")
        require(ui.scalar(capture.database, "SELECT count(*) FROM download_tasks") == 0, "A view-only fixture queued a source download")
        return {"fixture": fixture, "network_disabled": True, "download_tasks": 0}

    def login_fixture(self, capture: ui.Capture, fixture: str) -> dict[str, Any]:
        boundary = self.fixture_boundary(capture, fixture)
        login = capture.state["login"]
        if fixture == "login-loading":
            require(login["step"] == "qr" and login["busy"] and login["qr_size"] == [0, 0], "Loading Login exposed a usable QR")
        elif fixture == "login-expired":
            require(login["step"] == "qr" and not login["busy"] and login["status"] == self.labels["login_expired"],
                    "Expired Login omitted its explicit expired state")
        else:
            require(login["step"] == "code" and login["code_length"] == 0 and not login["can_submit"],
                    "The unfilled verification code was enabled")
        return {**boundary, "login_step": login["step"], "login_status": login["status"], "upstream_exercised": False}

    def download_fixture(self, capture: ui.Capture, fixture: str) -> dict[str, Any]:
        boundary = self.fixture_boundary(capture, fixture)
        shell = capture.state["shell"]
        if fixture == "detail-download":
            expected = self.labels["download_catalog_summary"].format(total=222, cached=3, selected=219)
            require(shell["dialog_kind"] == "download" and shell["dialog_summary"] == expected and shell["dialog_completed"] == "",
                    "Download options did not retain their decision summary")
        else:
            expected = self.labels["download_progress_completed"].format(done=12, total=219)
            require(shell["dialog_kind"] == "downloading" and shell["dialog_completed"] == expected and shell["dialog_summary"] == "",
                    "Running progress did not replace the decision summary with one completed line")
            require(abs(shell["dialog_progress"] - 12 / 219) < 0.0001, "The fixture's progress bar has inconsistent progress")
        return {**boundary, "visible_copy": expected, "dialog_kind": shell["dialog_kind"], "upstream_exercised": False}

    @staticmethod
    def cached_list_icon(capture: ui.Capture, profile: str) -> dict[str, Any]:
        books = capture.state["library"]["books"]
        require(bool(books) and books[0]["id"] == "demo-0" and books[0]["cached"], "The first list entry is not the cached fixture book")
        margin = 32 if profile == "scribe" else 20
        image = grayscale(capture.path)
        x0 = margin + 44 + 14
        # A 14px download icon has a centered vertical stem, a V and a separate
        # baseline. This checks the actual raster geometry before the author.
        matches = []
        for y0 in range(144, 186):
            # At 14/24 scale the centered SVG stem straddles two pixels, each
            # with approximately half coverage (127); its baseline stays black.
            stem = sum(image.getpixel((x0 + 7, y0 + dy)) <= 150 for dy in range(3, 9))
            baseline = sum(image.getpixel((x0 + dx, y0 + 12)) <= 100 for dx in range(3, 12))
            whitespace = sum(image.getpixel((x0 + dx, y0 + 1)) >= 240 for dx in range(2, 12))
            arrow = sum(image.getpixel((x0 + dx, y0 + dy)) <= 150 for dx, dy in ((4, 7), (9, 7), (6, 9), (7, 9)))
            if stem >= 5 and baseline >= 7 and whitespace >= 8 and arrow == 4:
                matches.append((x0, y0, 14, 14))
        require(bool(matches), "The cached list row has no actual download-icon silhouette before the author")
        return {"cached_book_id": books[0]["id"], "icon_candidates": matches}

    @staticmethod
    def qr_geometry(capture: ui.Capture, profile: str) -> dict[str, Any]:
        image = grayscale(capture.path)
        width, height = ui.PROFILES[profile]
        left = (width - 480) // 2 if profile == "scribe" else 40
        right = width - left
        top = 300 if profile == "scribe" else 150
        require(ink_ratio(image, (left + 12, top, right - 12, top + 2), maximum=100) > 0.99, "The QR card top border is not at the declared prototype anchor")
        bottom_rows = [y for y in range(top + 350, min(height - 30, top + 600)) if ink_ratio(image, (left + 12, y, right - 12, y + 1), maximum=100) > 0.99]
        require(bool(bottom_rows), "The content-sized QR card has no complete outer bottom border")
        bottom = max(bottom_rows) + 1
        # The handoff's 460/524px raster cards have different text/button metrics
        # from the common 48px controls. Bound the exact content stack below,
        # then record its actual outer dimensions for the paired visual review.
        require(bottom - top < 600, "The QR card retained the previous fixed 648px height")
        for x in (left, right - 1):
            require(ink_ratio(image, (x, top + 12, x + 1, bottom - 12), maximum=100) > 0.98, "The QR card outer sides do not frame its complete content")
        require(left + right == width, "The QR card is not horizontally centered")
        qr_box = (left + 28, top + 72, right - 28, top + 390) if profile == "scribe" else (left + 24, top + 64, right - 24, top + 338)
        qr_ink = image.crop(qr_box).point(lambda value: 255 if value <= 64 else 0).getbbox()
        require(qr_ink is not None, "The explicit offline QR fixture has no rendered symbol")
        symbol = [qr_box[0] + qr_ink[0], qr_box[1] + qr_ink[1], qr_box[0] + qr_ink[2], qr_box[1] + qr_ink[3]]
        require(abs((symbol[0] + symbol[2]) / 2 - width / 2) <= 1, "The actual QR symbol remains left-biased within its centered card")
        require(abs((symbol[2] - symbol[0]) - (symbol[3] - symbol[1])) <= 1, "The QR symbol was cropped or stretched")
        padding = 28 if profile == "scribe" else 24
        button_height = 52 if profile == "scribe" else 48
        close_top = bottom - padding - button_height
        require(any(ink_ratio(image, (left + padding + 10, y, right - padding - 10, y + 1), maximum=100) > 0.99 for y in (close_top - 1, close_top, close_top + 1)), "The content-sized card has unexpected blank space below its final action")
        return {"card_bounds": [left, top, right, bottom], "card_height": bottom - top, "qr_ink_bounds": symbol, "horizontal_center": width / 2, "bottom_padding": padding, "fixed_648px_height_removed": True}

    def reading(self, profile: str) -> None:
        prefix = f"design/reading/{profile}"
        stem = "SC" if profile == "scribe" else "PW"
        seed = self.run_case(f"{prefix}/seed", profile=profile, screen="reader", data_key=prefix)
        fixture = fixture_annotations(seed.database)
        sidebar_actions = [action(200, "reader", "thought-position", "side"), action(300, "reader", "menu", "false")] if profile == "scribe" else []
        default_reader = self.observe(f"{prefix}/default-reading", profile=profile, screen="reader", data_key=prefix)
        self.assertion(f"{prefix}/default-reading-retains-hidden-annotations", lambda: self.hidden_annotations(default_reader, fixture), default_reader)
        if profile == "scribe":
            self.assertion(f"{prefix}/default-reading-uses-the-former-sidebar-for-body-text", lambda: self.annotation_mode(default_reader, profile, panel=False), default_reader)
            annotations = self.observe(f"{prefix}/annotations", profile=profile, screen="reader", data_key=prefix, actions=sidebar_actions)
            self.assertion(f"{prefix}/cached-popular-review-preview-is-present", lambda: self.annotation_preview(annotations, fixture), annotations)
            self.assertion(f"{prefix}/annotation-sidebar-opens-only-when-requested", lambda: self.annotation_mode(annotations, profile, panel=True), annotations)
            self.pair(annotations, "SC-Reader.png", state="Reader populated annotation sidebar explicitly opened")
        thread = self.observe(f"{prefix}/thread", profile=profile, screen="reader", data_key=prefix, actions=[*sidebar_actions, action(400, "reader", "thought", "remote:0")])
        preview_check = "cached-popular-thread-preview-is-present" if profile == "scribe" else "cached-popular-review-preview-is-present"
        self.assertion(f"{prefix}/{preview_check}", lambda: self.opened_annotation(thread, fixture, mine=False), thread)
        if profile == "scribe":
            self.assertion(f"{prefix}/side-thread-restores-return-header", lambda: self.side_thread(thread), thread)
            self.pair(thread, "SC-Reader--thought.png", state="Reader expanded annotation thread in explicitly opened sidebar")
        mine_actions = [*sidebar_actions, action(400, "reader", "thought", f"mine:{fixture['note_id']}")]
        mine = self.observe(f"{prefix}/my-annotation", profile=profile, screen="reader", data_key=prefix, actions=mine_actions)
        self.assertion(f"{prefix}/my-annotation-opens-selected-note", lambda: self.opened_annotation(mine, fixture, mine=True), mine)
        close_actions = [*mine_actions, action(600, "reader", "thought-close")]
        if profile == "scribe":
            close_actions.append(action(800, "reader", "thought-position", "center"))
        closed = self.observe(f"{prefix}/closed-reading", profile=profile, screen="reader", data_key=prefix, actions=close_actions)
        self.assertion(f"{prefix}/closing-annotations-restores-counts-and-empty-model", lambda: self.hidden_annotations(closed, fixture), closed)
        for tab in ("marks", "notes"):
            capture = self.observe(f"{prefix}/toc-{tab}", profile=profile, screen="reader", data_key=prefix, actions=[action(200, "reader", "toc", tab)])
            self.assertion(f"{prefix}/toc-{tab}-contains-independent-location-fields", lambda c=capture, t=tab: self.reader_entries(c, t), capture)
            self.pair(capture, f"{stem}-Reader--toc-{tab}.png", state=f"Reader populated {tab}")
        for tool, subtool, suffix, heights in (
            ("progress", "", "progress", (152, 152)), ("notes", "", "notes", (200, 200)),
            ("light", "", "light", (272, 272)), ("type", "", "type", (384, 344)),
            ("type", "font", "type-font", (188, 308)), ("type", "indent", "type-indent", (168, 276)),
            ("type", "turn", "type-turn", (274, 386)),
        ):
            height = heights[0 if profile == "scribe" else 1]
            actions = [action(100, "reader", "menu", "true"), action(200, "reader", "tool", tool)]
            if subtool:
                actions.append(action(400, "reader", "subtool", subtool))
            capture = self.observe(f"{prefix}/{suffix}", profile=profile, screen="reader", actions=actions)
            self.assertion(f"{prefix}/{suffix}-tool-is-open", lambda c=capture, t=tool, s=subtool: self.reader_tool(c, t, s), capture)
            self.assertion(f"{prefix}/{suffix}-content-fit-and-five-tools",
                lambda c=capture, h=height: self.panel_geometry(c, profile, 5, h, tolerance=9), capture)
            if tool not in ("progress", "notes"):
                self.pair(capture, f"{stem}-Reader--{suffix}.png", state=f"Reader {suffix}")
        reader_toast = self.run_case(f"{prefix}/progress-offline-toast", profile=profile, screen="reader", actions=[
            action(150, "reader", "menu", "true"), action(350, "reader", "tool", "progress"), action(500, "reader", "sync"),
        ])
        nav = 76 if profile == "scribe" else 64
        self.assertion(f"{prefix}/reader-toast-follows-panel-height", lambda: ui.bottom_toast(reader_toast, nav + 152 + 16), reader_toast)
        for tool, height in (("", 0), ("images", 152)):
            events = [action(150, "article", "article.menu", "true")]
            if tool:
                events.append(action(300, "article", "article.tool", tool))
            panel = self.run_case(f"{prefix}/article-panel-{tool or 'tools-only'}", profile=profile, screen="article", actions=events)
            self.assertion(f"{prefix}/article-{tool or 'tools-only'}-has-no-extra-band",
                lambda c=panel, h=height: self.panel_geometry(c, profile, 4, h), panel)
        article_key = f"{prefix}/article-fixture"
        seed = self.run_case(f"{prefix}/article-seed", profile=profile, screen="article", data_key=article_key)
        fixture_article(seed.database, cached=False)
        placeholder = self.observe(f"{prefix}/article-placeholder", profile=profile, screen="article", data_key=article_key)
        self.assertion(f"{prefix}/article-placeholder-frame-size", lambda: self.article_frame(placeholder, profile, loaded=False), placeholder)
        payload = fixture_article(placeholder.database, cached=True)
        require(payload is not None, "The article bitmap fixture was not created")
        loaded = self.observe(f"{prefix}/article-loaded", profile=profile, screen="article", data_key=article_key)
        self.assertion(f"{prefix}/article-loaded-frame-size", lambda: self.article_frame(loaded, profile, loaded=True), loaded)
        self.assertion(f"{prefix}/article-real-bitmap-matches", lambda: self.article_bitmap(loaded, payload), loaded)
        serif_body = self.observe(f"{prefix}/article-serif-body", profile=profile, screen="article", data_key=article_key, actions=[action(200, "article", "article.font", "song")])
        self.assertion(f"{prefix}/quote-retains-its-independent-font", lambda: self.quote_independence(loaded, serif_body, profile), loaded, serif_body)
        self.pair(loaded, f"{stem}-Article--images.png", state="Article loaded inline picture", exclusions=["Fixture title, source text and article pagination", "The bitmap is an independent geometric test asset with the same 2.5:1 aspect ratio"])

    @staticmethod
    def annotation_mode(capture: ui.Capture, profile: str, *, panel: bool) -> dict[str, Any]:
        reader = capture.state["reader"]
        require(reader["notes_panel"] is panel, f"The Scribe annotation sidebar visibility is not the requested {panel} state")
        require(not reader["menu"] and not reader["toc_open"] and not reader["thought_open"] and not reader["selection_open"], "The reader layout fixture is obscured by another panel or selection")
        require(reader["page"] == 1, "The annotation layout fixture must begin on the first chapter page")
        width, height = ui.PROFILES[profile]
        image = grayscale(capture.path)
        require(image.size == (width, height), "The annotation layout capture does not use its declared logical viewport")
        boundary = width - 300
        divider_box = (boundary, 76, boundary + 1, height - 48)
        divider_ratio = ink_ratio(image, divider_box, maximum=190)
        if panel:
            require(reader["thought_position"] == "side", "The explicitly opened Scribe sidebar must retain its side placement")
            header_box = (boundary, 74, width, 76)
            header_ratio = ink_ratio(image, header_box, maximum=64)
            require(header_ratio > 0.99, "The explicitly opened annotation sidebar has no complete 300px header divider")
            # ThoughtCard paper backgrounds cover the outer divider within each
            # card. Inspect its long unobscured segment below the populated rows.
            cards_bottom = 76 + sum(164 if thought["quote"] else 112 for thought in reader["thoughts"])
            visible_divider_box = (boundary, cards_bottom + 1, boundary + 1, height - 48)
            require(visible_divider_box[3] - visible_divider_box[1] >= height // 4, "The annotation fixture has no substantial unobscured sidebar boundary")
            visible_divider_ratio = ink_ratio(image, visible_divider_box, maximum=190)
            require(visible_divider_ratio > 0.99, "The explicitly opened annotation sidebar has no continuous visible text/sidebar divider")
            return {"notes_panel": True, "explicit_open": True, "sidebar_width": 300, "header_divider_box": list(header_box), "header_divider_ink_ratio": header_ratio, "visible_divider_box": list(visible_divider_box), "visible_divider_ink_ratio": visible_divider_ratio, "full_height_divider_ink_ratio": divider_ratio}

        # Use the same populated chapter as the sidebar comparison. Several
        # separate text bands in its former sidebar area prove actual reflow,
        # rather than accepting a hidden panel that leaves a blank column.
        require(divider_ratio < 0.5, "Default reading retained the annotation sidebar's vertical divider")
        body_box = (boundary + 16, 204, width - 96, min(height - 48, 204 + reader["font_size"] * 10))
        require(body_box[2] > body_box[0] and body_box[3] > body_box[1], "The default reader fixture has no inspectable former-sidebar body area")
        body_ink = ink_ratio(image, body_box, maximum=100)
        require(body_ink > 0.02, "Default reading did not render body text across the former annotation sidebar")
        occupied_rows = [ink_ratio(image, (body_box[0], y, body_box[2], y + 1), maximum=100) > 0.02 for y in range(body_box[1], body_box[3])]
        text_bands = [(body_box[1] + start, body_box[1] + end) for start, end in runs(occupied_rows, 3)]
        require(len(text_bands) >= 3, f"The former sidebar area contains fewer than three distinct body-text bands: {text_bands}")
        return {"notes_panel": False, "full_width_reading_default": True, "former_sidebar_body_box": list(body_box), "body_ink_ratio": body_ink, "body_text_bands": text_bands, "divider_ink_ratio": divider_ratio}

    @staticmethod
    def hidden_annotations(capture: ui.Capture, fixture: dict[str, Any]) -> dict[str, Any]:
        reader = capture.state["reader"]
        require(not reader["notes_panel"] and not reader["thought_open"] and not reader["toc_open"]
            and not reader["menu"] and not reader["selection_open"] and not reader["writing_thought"], "The hidden annotation fixture has an open reader view")
        require(reader["thoughts"] == [] and reader["thought_count"] == 0 and reader["entries"] == []
            and reader["entry_count"] == 0 and reader["note_id"] == "", "A hidden annotation view retained text models or a selected note")
        require(reader["popular"] and reader["mine"], "The fixture's public and private annotation options are not enabled")
        require(reader["total_thought_count"] == 2 and reader["note_count"] == 1 and reader["mark_count"] == 1
            and reader["bookmarked"] and reader["thought_page"] == 1 and reader["thought_pages"] == 1, "The hidden reader lost its independent annotation, bookmark or thought counts")
        require(annotation_history(capture.database) == fixture["history"], "Opening or hiding annotations changed persisted notes, bookmarks, cached reviews or query history")
        markers = [(line, mark) for line in reader["lines"] for mark in line["marks"]]
        private_id = f"mine-row:{fixture['note_rowid']}"
        private = [(line, mark) for line, mark in markers if mark["id"] == private_id]
        remote = [(line, mark) for line, mark in markers if mark["id"] == "remote:0"]
        require(bool(private) and bool(remote) and {mark["id"] for _, mark in markers} == {private_id, "remote:0"}, "The hidden reader lost an exact fixture annotation marker or retained an unrelated marker")
        require(all(0 <= line["start"] < len(fixture["quote"]) and line["end"] > 0 and mark["x"] >= 0
            and mark["width"] > 0 and not mark["selected"] for line, mark in markers), "The hidden annotation markers have no valid quoted text geometry")
        require(all(line["mine"] and mark["mine"] and mark["count"] == 0 for line, mark in private)
            and all(line["highlight"] and not mark["mine"] and line["count"] == 12 for line, mark in remote)
            and max(mark["count"] for _, mark in remote) == 12, "The hidden annotation markers lost their private/public flags or popular count")
        return {"visible_thought_count": 0, "total_thought_count": reader["total_thought_count"],
            "note_count": reader["note_count"], "mark_count": reader["mark_count"], "persisted_history_preserved": True,
            "private_marker_id": private_id, "private_marker_segments": len(private), "remote_marker_segments": len(remote)}

    @staticmethod
    def annotation_preview(capture: ui.Capture, fixture: dict[str, Any]) -> dict[str, Any]:
        reader = capture.state["reader"]
        require(reader["notes_panel"] and not reader["thought_open"] and reader["popular"] and reader["mine"], "Annotation previews require the explicitly opened list with both options enabled")
        thoughts = capture.state["reader"]["thoughts"]
        remote = [thought for thought in thoughts if thought["id"] == "remote:0"]
        mine = [thought for thought in thoughts if thought["mine"]]
        require(len(remote) == 1 and bool(remote[0]["body"]), "The cached popular review preview was dropped from the actual reader model")
        require(bool(remote[0]["quote_lines"]), "The remote quote has no measured lines for its dashed underlines")
        require(len(mine) == 1 and bool(mine[0]["quote_lines"]), "The private local annotation has no independently measured quote lines")
        require(remote[0]["body"] == fixture["review_body"] and remote[0]["quote"] == fixture["quote"]
            and "".join(remote[0]["quote_lines"]) == fixture["quote"], "The public preview lost its exact cached body or quoted anchor")
        require(mine[0]["id"] == f"mine:{fixture['note_id']}" and mine[0]["body"] == fixture["note_body"]
            and mine[0]["quote"] == fixture["quote"] and "".join(mine[0]["quote_lines"]) == fixture["quote"], "The private preview lost its exact note body or independently measured quoted anchor")
        require(annotation_history(capture.database) == fixture["history"], "Rendering previews changed persisted annotation history")
        return {"remote_preview": remote[0]["body"], "remote_quote_lines": remote[0]["quote_lines"], "my_annotation_count": len(mine)}

    @staticmethod
    def opened_annotation(capture: ui.Capture, fixture: dict[str, Any], *, mine: bool) -> dict[str, Any]:
        reader = capture.state["reader"]
        require(reader["thought_open"] and reader["popular"] and reader["mine"] and not reader["menu"]
            and not reader["toc_open"] and not reader["selection_open"] and not reader["writing_thought"], "The requested real annotation thread is not open")
        require(reader["thought_title"] == fixture["quote"] and reader["selection_text"] == fixture["quote"], "The opened thread lost its exact quoted anchor")
        require(reader["thought_count"] == 1 and reader["total_thought_count"] == 1 and len(reader["thoughts"]) == 1, "The opened thread did not publish exactly its selected annotation family")
        entry = reader["thoughts"][0]
        expected_id = f"mine:{fixture['note_id']}" if mine else fixture["review_id"]
        expected_body = fixture["note_body" if mine else "review_body"]
        require(entry["id"] == expected_id and entry["mine"] is mine and entry["body"] == expected_body,
                "The opened annotation did not retain the exact selected preview body")
        require(reader["note_id"] == (fixture["note_id"] if mine else ""), "The opened thread selected an unrelated local note")
        if mine:
            require(entry["quote"] == fixture["quote"] and bool(entry["quote_lines"])
                and "".join(entry["quote_lines"]) == fixture["quote"], "The selected private annotation lost its independently measured quote lines")
        else:
            require(entry["quote"] == "" and entry["quote_lines"] == [], "The public review body incorrectly replaced its separate quoted header")
        require(annotation_history(capture.database) == fixture["history"], "Opening the selected annotation changed persisted history")
        return {"entry_id": entry["id"], "preview": entry["body"], "quoted_anchor": reader["thought_title"],
            "quote_lines": entry["quote_lines"], "selected_note_id": reader["note_id"], "persisted_history_preserved": True}

    @staticmethod
    def side_thread(capture: ui.Capture) -> dict[str, Any]:
        reader = capture.state["reader"]
        require(reader["notes_panel"] and reader["thought_open"] and reader["thought_position"] == "side", "The actual Scribe side thread is not open in the explicitly requested sidebar")
        image = grayscale(capture.path)
        require(ink_ratio(image, (632, 74, 928, 76), maximum=64) > 0.99, "The dedicated 76px return header divider is missing")
        require(ink_ratio(image, (646, 20, 850, 68), maximum=100) > 0.025, "The side-thread return action has no visible text and icon")
        return {"side_thread": True, "return_header_height": 76, "visible_return_action": True}

    @staticmethod
    def quote_independence(before: ui.Capture, after: ui.Capture, profile: str) -> dict[str, Any]:
        require(before.state["article"]["font_choice"] == "hei" and after.state["article"]["font_choice"] == "song", "The body font was not actually changed for the quote isolation check")
        margin = 72 if profile == "scribe" else 32
        top = 80 if profile == "scribe" else 54
        start: int | None = None
        end = top
        for block in before.state["article"]["blocks"]:
            if block["kind"] == "quote":
                if start is None:
                    start = top
                end = top + block["height"]
            elif start is not None:
                break
            top += block["height"]
        require(start is not None and end > start, "The independent quotation fixture is not visible")
        width = ui.PROFILES[profile][0]
        comparison = matching_capture(before, after, (margin, start, width - margin, end), maximum=0)
        change = ui.pixel_difference(before.path, after.path, threshold=8)
        require(change > 0.001, "Changing the article body font was a no-op")
        return {**comparison, "body_font_change_ratio": change, "quote_font_independent_of_body": True}

    @staticmethod
    def reader_entries(capture: ui.Capture, tab: str) -> dict[str, Any]:
        reader = capture.state["reader"]
        require(reader["toc_open"] and reader["toc_tab"] == tab, "The requested reader list is not open")
        require(len(reader["entries"]) == 1, f"The populated {tab} fixture was not rendered exactly once")
        entry = reader["entries"][0]
        require(bool(entry["chapter"]) and bool(entry["date"]), "The reader entry lost its independent chapter/date fields")
        require(tab != "marks" or bool(entry["page_label"]), "The bookmark lost its chapter page field")
        return {"tab": tab, "entry": entry}

    @staticmethod
    def reader_tool(capture: ui.Capture, tool: str, subtool: str) -> dict[str, Any]:
        reader = capture.state["reader"]
        require(reader["menu"] and reader["tool"] == tool and reader["subtool"] == subtool, "The intended reader panel is not visible")
        return {"tool": tool, "subtool": subtool}

    @staticmethod
    def panel_geometry(capture: ui.Capture, profile: str, tools: int, content_height: int, *, tolerance: int = 4) -> dict[str, Any]:
        image = grayscale(capture.path)
        row_top = image.height - (76 if profile == "scribe" else 64)
        expected = row_top - content_height
        rows = [y for y in range(expected - tolerance, expected + tolerance + 2)
                if ink_ratio(image, (4, y, image.width - 4, y + 1), maximum=64) > 0.99]
        require(bool(rows), f"Panel boundary near y={expected} did not fit its declared content height")
        scores = []
        for index in range(tools):
            x = round(image.width * (index + 0.5) / tools)
            scores.append(ink_ratio(image, (x - 21, row_top + 6, x + 21, image.height - 6), maximum=64))
        require(all(score > 0.015 for score in scores), f"Some of the {tools} evenly distributed toolbar targets are blank: {scores}")
        return {"panel_top": min(rows), "content_height": row_top - min(rows), "tool_slots": tools, "tool_ink_ratios": scores}

    @staticmethod
    def article_frame(capture: ui.Capture, profile: str, *, loaded: bool) -> dict[str, Any]:
        blocks = [block for block in capture.state["article"]["blocks"] if block["kind"] == "image"]
        require(len(blocks) == 1, "The fixture image is not visible exactly once on the current article page")
        expected = (300 if profile == "scribe" else 220) if loaded else (220 if profile == "scribe" else 180)
        require(blocks[0]["loaded"] is loaded, "The image cache state was not reflected in the real article block")
        require(blocks[0]["frame_height"] == expected, f"The article frame is {blocks[0]['frame_height']}px instead of {expected}px")
        return {"loaded": loaded, "frame_height": blocks[0]["frame_height"], "total_block_height": blocks[0]["height"]}

    @staticmethod
    def article_bitmap(capture: ui.Capture, payload: Path) -> dict[str, Any]:
        # The app prepares a display-sized raster before Slint receives it.
        # Keep the same strict pixel limits and compare at the model's exact origin.
        frame = grayscale(capture.path)
        source = grayscale(payload)
        profile = capture.metadata["profile"]
        margin = 72 if profile == "scribe" else 32
        target_width = frame.width - margin * 2
        target_height = 300 if profile == "scribe" else 220
        top = 80 if profile == "scribe" else 54
        for block in capture.state["article"]["blocks"]:
            if block["kind"] == "image":
                break
            top += block["height"]
        else:
            raise AssertionError("The actual article model contains no visible image block")
        require(top + target_height <= frame.height, "The article bitmap is clipped outside the viewport")
        expected, sampler = prepared_cover(source, (target_width, target_height))
        actual = frame.crop((margin, top, margin + target_width, top + target_height))
        histogram = ImageChops.difference(expected, actual).histogram()
        area = target_width * target_height
        mean_error = sum(value * count for value, count in enumerate(histogram)) / area
        difference_ratio = sum(histogram[21:]) / area
        require(mean_error <= 2.0 and difference_ratio <= 0.02, f"The center-cover article bitmap failed strict raster matching: mean={mean_error}, ratio={difference_ratio}")
        return {"bitmap_bounds": [margin, top, target_width, target_height], "bitmap_mean_error": mean_error, "bitmap_difference_ratio": difference_ratio, "exact_pixel_ratio": histogram[0] / area, "registration_search": False, "sampler": sampler, "image_fit": "center-cover"}

    def tasks(self) -> list[tuple[str, Callable[[], None]]]:
        tasks = [("design/catalog/scribe/primary", lambda: self.catalog_primary("scribe"))]
        for mode in ("cover", "list"):
            for profile in ("pw6", "scribe"):
                tasks.append((f"design/catalog/{profile}/{mode}", lambda p=profile, m=mode: self.catalog_mode(p, m)))
        for profile in ("pw6", "scribe"):
            tasks.append((f"design/catalog/{profile}/detail", lambda p=profile: self.catalog_detail(p)))
        for group, method in (("auth", self.auth), ("controls", self.controls), ("observed", self.observed), ("reading", self.reading), ("foundation", self.foundation)):
            for profile in ("pw6", "scribe"):
                name = f"design/{group}/{profile}"
                tasks.append((name, lambda p=profile, callback=method: callback(p)))
        if self.arguments.only:
            prefixes = tuple(part.strip().strip("/") for part in self.arguments.only.split(",") if part.strip())
            tasks = [(name, callback) for name, callback in tasks if any(name == prefix or name.startswith(prefix + "/") for prefix in prefixes)]
        require(bool(tasks), "No design regression groups match the requested filter")
        return tasks

    def run(self) -> int:
        status = super().run()
        report_path = self.output / "report.json"
        report = json.loads(report_path.read_text(encoding="utf-8"))
        report["suite"] = "Desktop Slint layout regressions and offline presentation fixtures"
        report["count_definition"] = "Counts include individual application captures and independently recorded assertions."
        report["verification_method_corrections"] = [
            "Locate input underlines from the actual search-field top border, rather than assuming vertically centered row children.",
            "Test the 14px SVG download stem's half-covered gray pixels and its separate black arrow/baseline geometry.",
            "Measure selected header backgrounds with continuous horizontal runs so inverted white glyphs do not split the box.",
            "Model current article picture preparation as an independent centered crop, triangle resize and Gray4 quantization; preserve mean<=2.0 and difference>20<=2% with no registration search.",
        ]
        report["paired_comparisons"] = self.pairs
        report["coverage_limits"].extend([
            "Named external UI overlays show QR/code/review layouts only and cannot establish successful authentication.",
            "Design pairs use a fixed 0.5 reference scale; text, dates, counts and QR payloads are not whole-screen pixel baselines.",
            "No physical Kindle model, firmware, panel refresh, EMR, multi-touch or live entitlement was exercised.",
        ])
        if self.arguments.state_harness is not None:
            report["state_harness"] = str(self.arguments.state_harness)
            report["state_harness_bytes"] = self.arguments.state_harness.stat().st_size
        report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
        return status


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--state-harness", type=Path, help="Separately built disposable offline fixture runner")
    parser.add_argument("--reference-dir", type=Path, help="Original v0.2 screens directory at 2px/dp")
    parser.add_argument("--output-dir", type=Path, default=Path("artifacts/design-verification"))
    parser.add_argument("--data-dir", type=Path)
    parser.add_argument("--workers", type=int, choices=(1, 2, 3), default=2)
    parser.add_argument("--only", help="Comma-separated prefixes such as design/catalog or design/auth/pw6")
    arguments = parser.parse_args()
    if os.name != "posix":
        parser.error("Run this harness only on the designated remote Linux worker through ssh test-env")
    arguments.baseline_dir = None
    arguments.max_diff_ratio = 0.01
    if arguments.state_harness is not None:
        arguments.state_harness = arguments.state_harness.resolve()
        require(arguments.state_harness.is_file(), "The offline fixture runner does not exist")
    if arguments.reference_dir is not None:
        arguments.reference_dir = arguments.reference_dir.resolve()
    try:
        return DesignVerifier(arguments).run()
    except Exception as error:
        print(f"Design regression setup failed: {ui.redact(str(error))}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
