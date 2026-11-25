"""Native handoff walkthrough used by verify-firmware-emulator.py.

Each group owns a fresh app launch in the same disposable offline data root.
QMP screenshots show scanout; final app captures independently identify the
route and native backend. Scripted setup intents are recorded as such.
"""

from __future__ import annotations

from contextlib import closing
import json
from pathlib import Path
import secrets
import sqlite3
import subprocess
import tempfile
import time

from PIL import Image, ImageChops


class Blocked(RuntimeError):
    """The guest cannot provide a required environmental observation."""


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def ink(image, box, maximum=190):
    area = image.crop(box)
    return sum(area.histogram()[:maximum + 1]) / (area.width * area.height)


def difference(before, after, box):
    change = ImageChops.difference(before.crop(box), after.crop(box))
    return sum(change.histogram()[17:]) / (change.width * change.height)


def seed_fixture(database, scribe):
    """Extend a clean demo database created by the running native application."""
    with closing(sqlite3.connect(database)) as connection, connection:
        def read(key):
            row, = connection.execute("SELECT value FROM settings WHERE key=?", (key,))
            return json.loads(row[0])

        def write(key, value):
            connection.execute("INSERT OR REPLACE INTO settings VALUES(?,?)", (key, json.dumps(value)))

        title, intro = connection.execute("SELECT title,intro FROM books WHERE id='demo-0'").fetchone()
        term = title[:2]
        connection.executemany(
            "INSERT INTO books(id,title,author,category,intro,cover_url,source) VALUES(?,?,?,?,?,?,?)",
            ((f"walkthrough-book-{index}", f"{term} · Walkthrough {index:02d}", "Offline fixture", "Demo", intro, "", "demo")
             for index in range(1, 28)),
        )
        connection.executemany(
            "INSERT INTO books(id,title,author,category,intro,cover_url,source,on_shelf) VALUES(?,?,?,?,?,?,?,0)",
            ((f"walkthrough-add-{index}", f"Addable walkthrough item {index}", "Offline fixture", "Demo", intro, "", "demo")
             for index in (1, 2)),
        )
        connection.executemany(
            "INSERT INTO query_history VALUES(?,?)", ((title, 1), (term, 2)),
        )
        connection.executemany(
            "INSERT INTO chapters(book_id,chapter_index,source_id,title,body) VALUES('demo-0',?,?,?,NULL)",
            ((index, f"walkthrough-chapter-{index}", f"Offline chapter {index + 1:02d}") for index in range(3, 25)),
        )
        connection.execute("UPDATE books SET total_chapters=25,intro=? WHERE id='demo-0'", ((intro * 7) if scribe else intro,))
        body, = connection.execute("SELECT body FROM chapters WHERE book_id='demo-0' AND chapter_index=0").fetchone()
        now = 1_793_664_000
        connection.execute(
            "INSERT INTO annotations VALUES('walkthrough-note','demo-0',0,20,42,?,?,?)",
            (body[20:42], "A local note linked to the demo chapter.", now),
        )
        connection.execute("INSERT INTO bookmarks VALUES('walkthrough-bookmark','demo-0',0,0,?,?)", ("Offline bookmark", now))
        write("underlines:demo-0:demo-chapter-0", [{"range": "0-12", "text": body[:12], "count": 32, "thoughts": 12}])
        write("thoughts:demo-0:demo-chapter-0:remote:0", [
            {"id": f"walkthrough-review-{index}", "author": f"Offline reader {index:02d}",
             "text": f"Cached public discussion entry {index:02d} for the walkthrough.",
             "likes": 12 - index, "published_at": now, "recommended": True}
            for index in range(12)
        ])
        accounts = read("mp.accounts")
        base_date = accounts[0]["updated_at"]
        accounts.extend({"id": f"walkthrough-account-{index}", "name": f"Offline account {index:02d}",
                         "cover_url": "", "updated_at": base_date - index, "unread_count": 0}
                        for index in range(1, 17))
        write("mp.accounts", accounts)
        feeds = read("mp.articles")
        articles = feeds["demo-publication"]
        articles[0]["original_url"] = "https://example.invalid/emulator-walkthrough"
        articles.extend({"id": f"walkthrough-article-{index}", "account_id": "demo-publication",
                         "title": f"Offline article {index:02d}", "author": "Offline fixture",
                         "published_at": base_date - 365 * 86_400 - index, "original_url": "", "cover_url": ""}
                        for index in range(1, 15))
        write("mp.articles", feeds)
        write("article.html:demo-article-0", (
            "<p>A short offline article exercises the image placeholder and the reading footer.</p>"
            '<img src="https://example.invalid/walkthrough-image.png" alt="Offline image placeholder">'
            "<p>This article uses invented content and never contacts an upstream service.</p>"
        ))


class Walkthrough:
    # Values are: initial app route, intended final route, optional setup intents.
    GROUPS = {
        "library": ("library", "library", ()),
        "detail": ("detail", "detail", ()),
        "detail-notes-link": ("detail", "reader", (("detail", "tab", "intro"),)),
        "search": ("search", "search", ()),
        "search-toasts": ("search", "search", (("search", "edit-query", "Addable walkthrough item"),)),
        "reader": ("reader", "detail", ()),
        "reader-notes": ("reader", "reader", (("reader", "thought-position", "center"), ("reader", "thought", "remote:0"))),
        "reader-sidebar": ("reader", "reader", (("reader", "thought-position", "side"), ("reader", "menu", "false"))),
        "reader-contents": ("reader", "reader", (("reader", "toc", "toc"),)),
        "publications": ("mp", "mp", ()),
        "article": ("article", "article", ()),
        "settings": ("settings", "settings", ()),
        "stats": ("library", "stats", ()),
        "sleep": ("reader", "reader", ()),
        "login": ("login", "login", ()),
    }

    def __init__(self, args, profile, directory, record, guest, qmp, capture, restored, read_pid, logs, lock, entry):
        self.args = args
        self.profile = profile
        self.root = directory
        self.record = record
        self.guest = guest
        self.qmp = qmp
        self.capture = capture
        self.restored = restored
        self.read_pid = read_pid
        self.logs = logs
        self.lock = lock
        self.entry = entry
        self.scribe = args.device_profile == "scribe"
        self.w, self.h = profile["logical_size"]
        self.margin, self.nav = (32, 76) if self.scribe else (20, 64)
        self.case_file = f"{logs}/test-case"
        self.workspace_file = f"{logs}/test-walkthrough"
        self.workspace = f"{logs}/ui-walkthrough-{secrets.token_hex(6)}"
        self.process = None

    def run(self):
        excluded = "detail-notes-link" if self.scribe else "reader-sidebar"
        available = tuple(group for group in self.GROUPS if group != excluded)
        groups = list(dict.fromkeys(group.strip() for group in self.args.only.split(","))) if self.args.only else available
        require(all(group in available for group in groups), f"Select groups from {', '.join(available)}")
        self.record.update({
            "input_and_captures": "Native Kindle binary under the guest launcher. QMP taps and QEMU scanout PNGs; any starting routes or timed setup intents are labeled separately.",
            "acceptance": "Targeted structural and interaction checks, not whole-screen pixel comparison or physical Kindle acceptance.",
            "groups_not_requested": [group for group in available if group not in groups],
            "fixture": "Built-in demo (offline) plus bounded, synthetic shelf, chapter, note, publication, and article records in fresh per-run guest state; Login uses separate empty, unauthenticated state.",
            "not_covered": {
                "detail": "Live reviews and an enabled download, including confirmation/progress/completion: desktop display fixtures only. Native uses disabled offline downloads and real local selection.",
                "search": "Live store results and entitlements; offline local shelf matches are used.",
                "reader": "Live public threads, synchronization, and physical light/EMR; public threads use seeded cached data. Scribe pen settings remain conditional on reported capability.",
                "publications_and_article": "Authenticated synchronization, download and image transfer; the seeded article uses an offline placeholder and a public synthetic phone-link QR.",
                "sleep": "No authenticated cover bitmap on the guest. The actual guest-power test requires a managed application cover acknowledgement; the bitmap and neutral layouts are desktop fixtures.",
                "login": "Service sign-in QR, expiration and verification code require a separate mock presentation or live account. Native uses offline Login and the app's actual Skills guide.",
                "hardware": "This run cannot establish real-device compatibility, panel behavior, EMR, or live services.",
            },
            "groups": [],
        })
        self.guest("test", "!", "-d", self.lock)
        prior = {path: self.guest("cat", path, check=False) for path in (self.case_file, self.workspace_file)}
        self.guest("mkdir", self.workspace)
        try:
            self.guest("tee", self.workspace_file, data=(self.workspace.rsplit("/", 1)[1] + "\n").encode())
            if any(group != "login" for group in groups):
                self.run_group("seed", "library", "library", (), self.seed)
                require(self.record["groups"][-1]["result"] == "passed", "Could not create the isolated demo state")
                guest_database = f"{self.workspace}/demo/library.sqlite"
                with tempfile.TemporaryDirectory(prefix="weread-firmware-ui-") as temp:
                    database = Path(temp) / "library.sqlite"
                    database.write_bytes(self.guest("cat", guest_database).stdout)
                    # SIGTERM can leave committed pages in the WAL after native exit.
                    wal = self.guest("cat", f"{guest_database}-wal", check=False)
                    if wal.returncode == 0 and wal.stdout:
                        database.with_name(f"{database.name}-wal").write_bytes(wal.stdout)
                    seed_fixture(database, self.scribe)
                    with sqlite3.connect(database) as connection:
                        connection.execute("PRAGMA wal_checkpoint(TRUNCATE)").fetchone()
                    connection.close()
                    self.guest("rm", "-f", f"{guest_database}-wal", f"{guest_database}-shm")
                    self.guest("tee", guest_database, data=database.read_bytes())
            for group in groups:
                start, end, actions = self.GROUPS[group]
                self.run_group(group, start, end, actions, getattr(self, group.replace("-", "_")))
        finally:
            if self.guest("test", "!", "-d", self.lock, check=False).returncode == 0:
                for path, original in prior.items():
                    if original.returncode == 0:
                        self.guest("tee", path, data=original.stdout)
                    else:
                        self.guest("rm", "-f", path)
                self.guest("rm", "-rf", self.workspace)
            else:
                self.record["cleanup_blocked"] = "Guest launcher still holds its lock; isolated fixture left in place."
        return all(group["result"] == "passed" for group in self.record["groups"])

    def run_group(self, name, start, end, actions, callback):
        self.directory = self.root / name
        self.directory.mkdir()
        self.inputs = []
        self.group = {"group": name, "initial_route": start, "expected_final_route": end, "checkpoints": []}
        self.record["groups"].append(self.group)
        started = time.monotonic()
        try:
            self.start(start, actions)
            callback()
            self.group["result"] = "passed"
        except Blocked as error:
            self.group.update(result="blocked", error=str(error))
        except Exception as error:
            self.group.update(result="failed", error=str(error))
        finally:
            try:
                self.finish(end)
            except Exception as error:
                self.group["finish_error"] = str(error)
                if self.group["result"] == "passed":
                    self.group["result"] = "failed"
            finally:
                if self.inputs:
                    self.group["inputs_after_last_checkpoint"] = self.inputs
                self.group["elapsed_seconds"] = round(time.monotonic() - started, 2)
                (self.directory / "result.json").write_text(json.dumps(self.group, indent=2) + "\n")
                error = self.group.get("error") or self.group.get("finish_error")
                suffix = f" — {error}" if error else ""
                print(f"{self.group['result'].upper()} {self.args.device_profile}/{name}: "
                      f"{len(self.group['checkpoints'])} QMP checkpoints{suffix}", flush=True)

    def start(self, route, actions):
        self.process = None
        self.guest("test", "!", "-d", self.lock)
        self.guest("rm", "-f", f"{self.workspace}/native.pid", f"{self.workspace}/final-native.png",
                   f"{self.workspace}/final-native.metadata.json", f"{self.workspace}/logs/application.log",
                   f"{self.workspace}/actions.json")
        if actions:
            setup = [dict(at_ms=300 + index * 250, section=section, action=action, value=value)
                     for index, (section, action, value) in enumerate(actions)]
            payload = json.dumps(setup, indent=2).encode()
            self.group["scripted_setup"] = setup
            (self.directory / "actions.json").write_bytes(payload)
            self.guest("tee", f"{self.workspace}/actions.json", data=payload)
        self.guest("tee", self.case_file, data=f"walkthrough-{route}\n".encode())
        with (self.directory / "launcher.log").open("wb") as output:
            self.process = subprocess.Popen(
                self.entry,
                stdout=output, stderr=subprocess.STDOUT,
            )
        def first_frame_ready():
            return self.guest("sh", "-c", "'test -s \"$1\" && grep -q first_application_frame_send \"$2\"'",
                              "sh", f"{self.workspace}/native.pid", f"{self.logs}/application-bootstrap.log",
                              check=False).returncode == 0

        self.wait_for(first_frame_ready, 100, "No native first-frame submission was observed under the launcher")
        time.sleep(1.2 if actions else 0.6)

    def finish(self, expected_route):
        if self.process is None:
            return
        try:
            if self.process.poll() is None:
                pid = self.guest("cat", f"{self.workspace}/native.pid", check=False).stdout.strip()
                owned = self.guest("cat", f"{self.lock}/application-pid", check=False).stdout.strip()
                if pid == owned and pid.isdigit() and int(pid) > 1:
                    self.guest("kill", "-TERM", int(pid), check=False)
                elif self.guest("test", "-s", f"{self.lock}/pid", check=False).returncode == 0:
                    self.guest("kill", "-TERM", self.read_pid(f"{self.lock}/pid"), check=False)
            status = self.process.wait(timeout=35)
            self.group["exit_code"] = status
            self.restored()
            # Service status can recover before the stock window finishes mapping.
            time.sleep(1.5)
            self.capture(self.directory, "restored-home")
            raw = self.guest("cat", f"{self.workspace}/final-native.metadata.json").stdout
            (self.directory / "native.metadata.json").write_bytes(raw)
            self.group["native_metadata"] = metadata = json.loads(raw)
            (self.directory / "native.png").write_bytes(self.guest("cat", f"{self.workspace}/final-native.png").stdout)
            require(status == 0, f"Native launcher exited with {status}")
            require(metadata["route"] == expected_route, f"Final native route is {metadata['route']}, not {expected_route}")
            require(metadata["backend"] == "kindle" and metadata["profile"] == self.args.device_profile, metadata)
            require(metadata["design_logical_size"] == self.profile["design_size"] and metadata["logical_size"] == self.profile["logical_size"], metadata)
            require(metadata["window_size"] == metadata["snapshot_size"] == list(self.profile["physical_size"]) and metadata["scale_factor"] == 2, metadata)
        finally:
            if self.process.poll() is None:
                try:
                    self.guest("kill", "-TERM", self.read_pid(f"{self.lock}/pid"), check=False)
                    self.process.wait(timeout=15)
                except Exception:
                    pass
            native = self.guest("cat", f"{self.workspace}/logs/application.log", check=False)
            bootstrap = self.guest("cat", f"{self.logs}/application-bootstrap.log", check=False)
            (self.directory / "application.log").write_bytes(native.stdout + native.stderr)
            (self.directory / "application-bootstrap.log").write_bytes(bootstrap.stdout + bootstrap.stderr)

    def wait_for(self, predicate, seconds, message):
        until = time.monotonic() + seconds
        last_transport_error = None
        while time.monotonic() < until:
            if self.process is not None and self.process.poll() is not None:
                raise RuntimeError(f"Application exited with {self.process.returncode}: {message}")
            try:
                if predicate():
                    return
            except subprocess.TimeoutExpired as error:
                last_transport_error = error
            time.sleep(0.35)
        raise TimeoutError(message) from last_transport_error

    def tap(self, x, y, label, delay=1.1):
        require(0 <= x < self.w and 0 <= y < self.h, (x, y, label))
        pixels = [round(2 * x), round(2 * y)]
        self.inputs.append({"input": "QMP tap", "target": label, "physical_px": pixels})
        self.qmp("tap", *pixels, "--hold", 0.1)
        time.sleep(delay)

    def checkpoint(self, name, ready=None, **observations):
        until = time.monotonic() + 6
        while True:
            path = self.capture(self.directory, name)
            frame = self.logical(path)
            if ready is None or ready(frame) or time.monotonic() >= until:
                break
            time.sleep(0.15)
        self.group["checkpoints"].append({
            "state": name, "qmp_capture": str(path.relative_to(self.root)),
            "inputs_since_previous_capture": self.inputs, **observations,
        })
        self.inputs = []
        return frame

    def logical(self, path):
        with Image.open(path) as frame:
            return frame.convert("L").resize((self.w, self.h), Image.Resampling.NEAREST)

    def observed(self, **measurements):
        self.group["checkpoints"][-1].update(measurements)

    def moved(self, before, after, box, minimum=0.002):
        value = difference(before, after, box)
        require(value > minimum, f"No expected UI change in {box}: {value:.3%}")
        self.observed(changed_region=list(box), changed_ratio=round(value, 5))

    def unchanged(self, before, after, box, fraction=0.001):
        value = difference(before, after, box)
        require(value <= fraction, f"Unexpected UI change in {box}: {value:.3%}")
        self.observed(unchanged_region=list(box), unchanged_ratio=round(value, 5))

    def pager_ink(self, frame, right, y):
        # The three distinct glyph groups reside in the declared 48/56/48 targets.
        left = right - 152
        targets = ((left, left + 48), (left + 48, right - 48), (right - 48, right))
        return [ink(frame, (a + 8, y - 15, b - 8, y + 15), maximum=170) for a, b in targets]

    def pager(self, frame, right, y):
        left = right - 152
        values = self.pager_ink(frame, right, y)
        require(all(value > 0.01 for value in values), f"Header pager does not show all three glyph groups: {values}")
        self.observed(pager_box=[left, y - 24, right, y + 24], pager_segment_ink=[round(value, 4) for value in values])

    def dialog(self, frame, *, top=None):
        left, right = (self.w - 580) // 2, (self.w + 580) // 2
        if not self.scribe:
            left, right = self.margin, self.w - self.margin
        ys = range((top - 1) if top is not None else self.h // 2 - 185,
                   (top + 3) if top is not None else self.h // 2 + 185)
        borders = [y for y in ys if ink(frame, (left + 10, y, right - 10, y + 1), maximum=94) > 0.99]
        require(bool(borders), "No expected centered dialog border is visible")
        self.observed(dialog_border_rows=borders)

    def nav_active(self, frame, index):
        center = round(self.w * (index + 0.5) / 4)
        box = (center - 24, self.h - self.nav + 12, center + 24, self.h - 12)
        require(ink(frame, box, maximum=64) > 0.62, f"Bottom navigation item {index + 1} is not selected")
        self.observed(active_navigation=index + 1)

    def rows(self, frame, top, count, height=72, left=None):
        x0 = self.margin + 12 if left is None else left + 6
        x1 = self.w - self.margin - 12
        expected = [top + (row + 1) * height - 1 for row in range(count)]
        for y in expected:
            require(any(ink(frame, (x0, at, x1, at + 1)) > 0.94 for at in (y - 1, y, y + 1)),
                    f"Missing list row divider near logical y={y}")
        self.observed(row_count_checked=count, row_dividers=expected)

    def toast_checkpoint(self, name, bottom):
        y = self.h - bottom - 52
        area = (self.margin + 12, y, self.w - self.margin - 12, y + 2)
        lower = (area[0], y + 50, area[2], y + 52)
        path = None
        seen = False
        # A QMP tap returns before asynchronous e-ink press/release updates settle.
        until = time.monotonic() + 4
        while time.monotonic() < until:
            path = self.capture(self.directory, name)
            frame = self.logical(path)
            if ink(frame, area, maximum=64) > 0.90 and ink(frame, lower, maximum=64) > 0.90:
                seen = True
                break
            time.sleep(0.12)
        self.group["checkpoints"].append({
            "state": name, "qmp_capture": str(path.relative_to(self.root)),
            "inputs_since_previous_capture": self.inputs,
        })
        self.inputs = []
        require(seen, f"Expected toast bounds at y={y} and bottom offset {bottom}")
        self.observed(toast_box=[self.margin, y, self.w - self.margin, y + 52], toast_bottom_offset=bottom)

    def panel(self, frame, tools, active, minimum, maximum):
        tool_top = self.h - self.nav
        bands = [y for y in range(max(80, tool_top - maximum), tool_top - minimum + 1)
                 if ink(frame, (2, y, self.w - 2, y + 1), maximum=94) > 0.98]
        require(bool(bands), f"No full-width panel rule in expected height range {minimum}–{maximum}")
        scores = []
        for index in range(tools):
            center = round(self.w * (index + 0.5) / tools)
            scores.append(ink(frame, (center - 22, tool_top + 5, center + 22, self.h - 5), maximum=64))
        require(all(score > 0.015 for score in scores), f"Missing rendered toolbar glyphs: {scores}")
        require([index for index, score in enumerate(scores) if score > 0.6] == ([] if active is None else [active]),
                f"Incorrect toolbar selection: {scores}; expected {active}")
        top = min(bands)
        self.observed(panel_top=top, panel_content_height=tool_top - top, visible_tool_slots=tools, active_tool_index=active)
        return top

    def tool_tap(self, count, index, label):
        self.tap(self.w * (index + 0.5) / count, self.h - self.nav / 2, label)

    def seed(self):
        self.nav_active(self.checkpoint("built-in-demo"), 0)

    def library(self):
        right = self.w - 8
        body = (self.margin, 128, self.w - self.margin, self.h - self.nav)
        grid = self.checkpoint("01-cover-grid")
        self.nav_active(grid, 0)
        self.pager(grid, right, 104)
        self.tap(right - 128, 104, "previous at first page")
        self.unchanged(grid, self.checkpoint("02-first-page-previous-inert"), body)
        self.tap(right - 24, 104, "next library page")
        second = self.checkpoint("03-grid-page-two")
        self.moved(grid, second, body, 0.01)
        self.tap(right - 76, 104, "library page label / jump")
        self.dialog(self.checkpoint("04-jump-dialog"))
        self.tap(3, 20, "dismiss jump dialog")
        self.tap(self.w - 53, 54, "enable cover selection on page two")
        self.tap(100, 260, "select first cover on page two")
        cover_selection = self.checkpoint("05-cover-selected-on-page-two")
        self.moved(second, cover_selection, (0, self.h - self.nav, self.w, self.h))
        self.unchanged(second, cover_selection, (right - 152, 80, right, 128))
        self.tap(self.w - 53, 54, "finish cover selection")
        self.tap(self.w - (151 if self.scribe else 130), 54, "list view")
        divider = (self.margin + 12, 198, self.w - self.margin - 12, 201)
        listing = self.checkpoint("06-list", ready=lambda frame: any(
            ink(frame, (divider[0], y, divider[2], y + 1)) > 0.94 for y in range(divider[1], divider[3])))
        self.moved(grid, listing, body, 0.01)
        row_count = 14 if self.scribe else 9
        self.rows(listing, 128, row_count)
        self.pager(listing, right, 104)
        self.tap(self.w - 53, 54, "enable list selection")
        self.tap(100, 164, "select first list row")
        self.tap(100, 236, "select second list row")
        selection = self.checkpoint("07-two-list-rows-selected")
        self.rows(selection, 128, row_count)
        self.moved(listing, selection, (0, self.h - self.nav, self.w, self.h))
        badges = [ink(selection, (self.margin + 1, y + 7, self.margin + 13, y + 19), maximum=64)
                  for y in (128, 200, 272)]
        require(min(badges[:2]) > 0.55 and badges[2] < 0.15, f"The first two list rows were not checked independently: {badges}")
        self.observed(checked_list_rows=[1, 2], badge_ink_ratios=badges)
        self.tap(self.w - (247 if self.scribe else 186), 54, "offline sync while selecting", delay=0.18)
        self.toast_checkpoint("08-selection-toast", self.nav + 16)
        self.tap(self.w - 53, 54, "finish selection")
        time.sleep(2)
        # Downloaded selects the one preloaded demo book on either profile.
        self.tap(332 if self.scribe else 312, 104, "downloaded sort")
        single = self.checkpoint("09-downloaded-one-page")
        self.pager(single, right, 104)
        self.tap(right - 76, 104, "single-page label is inert")
        self.unchanged(single, self.checkpoint("10-single-page-inert"), (0, 28, self.w, self.h))
        self.tap(402 if self.scribe else 382, 104, "Finished filter (empty shelf)")
        empty = self.checkpoint("11-empty-finished-filter")
        self.pager(empty, right, 104)
        self.moved(single, empty, body, 0.005)

    def detail(self):
        y = 176 if self.scribe else 370
        right = self.w - self.margin
        controls_left = 336 if self.scribe else self.margin
        tab_y = 128 if self.scribe else 322
        body = (controls_left, y + 24, right, self.h - 24)
        overview = self.checkpoint("01-scribe-introduction-and-contents" if self.scribe else "01-default-contents")
        if not self.scribe:
            self.tap(self.w / 6, tab_y, "Introduction tab")
            before = self.checkpoint("02-introduction")
            self.moved(overview, before, (self.margin, tab_y + 24, right, self.h - 24))
            self.tap(self.w / 2, tab_y, "Contents tab")
            overview = self.checkpoint("03-contents")
        self.pager(overview, right, y)
        if self.scribe:
            links = [at for at in range(1080, 1216) if ink(overview, (38, at, 70, at + 1), maximum=64) > 0.94]
            require(bool(links), "The synthetic long introduction has no visible Full Text underline")
            self.tap(58, max(links) - 12, "Full introduction")
            self.dialog(self.checkpoint("03-full-introduction-dialog"), top=160)
            self.tap(3, 110, "dismiss full introduction inside Detail")
        self.tap(right - 24, y, "next chapters page")
        second = self.checkpoint("04-contents-page-two")
        self.moved(overview, second, body, 0.005)
        self.tap(right - 128, y, "previous chapters page")
        self.tap(controls_left + 30, y, "enter chapter selection")
        empty = self.checkpoint("05-select-zero")
        self.moved(overview, empty, (4, 28, self.w - 4, 80))
        self.tap(controls_left + 40, y, "select all uncached chapters on this page")
        picked = self.checkpoint("06-select-this-page")
        # Only the centered number changes; the offline action stays disabled.
        self.moved(empty, picked, (self.w // 2 - 60, 28, self.w // 2 + 60, 80))
        self.tap(self.w - 64, 54, "offline selected download is disabled")
        self.unchanged(picked, self.checkpoint("07-disabled-download"), (0, 28, self.w, self.h))
        self.tap(38, 54, "finish chapter selection")
        self.tap(617 if self.scribe else self.w * 5 / 6, tab_y, "Reviews tab")
        review = self.checkpoint("08-offline-recommended-reviews")
        review_y = 184 if self.scribe else 372
        self.pager(review, right, review_y)
        self.tap(controls_left + 160, review_y, "latest reviews")
        self.moved(review, self.checkpoint("09-offline-latest-reviews"), (controls_left, review_y - 24, right, review_y + 24))
        if self.scribe:
            self.tap(804, tab_y, "My notes tab")
            self.moved(review, self.checkpoint("10-populated-my-notes"), (controls_left, tab_y + 24, right, 440))
        self.tap(self.w - 32, 54, "Detail more menu")
        self.moved(review, self.checkpoint("11-more-menu"), (self.w - 260, 80, self.w - 8, 310))
        self.tap(3, 20, "dismiss Detail more")

    def detail_notes_link(self):
        intro = self.checkpoint("01-paperwhite-introduction")
        self.tap(self.w / 2, 498, "My Notes link from the Paperwhite Introduction")
        notes = self.checkpoint("02-reader-local-notes")
        self.moved(intro, notes, (self.margin, 80, self.w - self.margin, 330), 0.01)
        self.pager(notes, self.w - self.margin, 114)

    def search(self):
        top = self.h - 300
        right = self.w - 8 - (self.margin if self.scribe else 0)
        keyboard = self.checkpoint("01-recents-and-keyboard")
        require(ink(keyboard, (2, top, self.w - 2, top + 2), maximum=64) > 0.95, "Search keyboard divider missing")
        self.tap(149 if self.scribe else self.w * 0.73, 112, "shelf scope")
        self.tap(self.w / 2, top + 144, "pinyin g")
        self.tap(self.w * 0.65, top + 84, "pinyin u")
        composing = self.checkpoint("02-inline-pinyin-candidates")
        self.moved(keyboard, composing, (self.margin + 44, 35, self.w - 110, 78))
        self.tap(self.margin + 27, 236, "open first recent query", delay=1.5)
        results = self.checkpoint("03-shelf-results-keyboard-closed")
        self.pager(results, right, 112)
        self.rows(results, 137, 15 if self.scribe else 9)
        self.tap(right - 24, 112, "next search result page")
        second = self.checkpoint("04-results-page-two")
        self.moved(results, second, (self.margin, 137, self.w - self.margin, top))
        self.tap(right - 76, 112, "search jump")
        self.dialog(self.checkpoint("05-results-jump"))
        self.tap(3, 20, "dismiss search jump")
        self.tap(220, 57, "focus query again")
        focused = self.checkpoint("06-results-keyboard-reopened")
        self.rows(focused, 137, 11 if self.scribe else 5)
        require(ink(focused, (2, top, self.w - 2, top + 2), maximum=64) > 0.95, "Search keyboard did not reopen")

    def search_toasts(self):
        result = self.checkpoint("01-local-store-results-with-keyboard")
        right = self.w - 8 - (self.margin if self.scribe else 0)
        self.pager(result, right, 112)
        self.tap(self.w - self.margin - 45, 173, "add the first synthetic item", delay=0.18)
        self.toast_checkpoint("02-toast-above-keyboard", 316)
        time.sleep(2)
        self.tap(self.w - 56, self.h - 34, "submit and close the touch keyboard")
        closed = self.checkpoint("03-local-store-results-no-keyboard")
        self.moved(result, closed, (0, self.h - 300, self.w, self.h), 0.01)
        self.tap(self.w - self.margin - 45, 245, "add the second synthetic item", delay=0.18)
        self.toast_checkpoint("04-toast-without-keyboard", 24)

    def reader(self):
        self.checkpoint("01-reading")
        self.tap(self.w / 2, self.h / 2, "open Reader menu")
        progress = self.checkpoint("02-progress")
        panel_top = self.panel(progress, 5, 1, 125, 180)
        self.tap(self.w - self.margin - 68, self.h - self.nav - 16 - 24, "offline reading progress sync", delay=0.18)
        self.toast_checkpoint("03-progress-toast", self.h - panel_top + 16)
        time.sleep(2)
        self.tool_tap(5, 2, "thought settings")
        self.panel(self.checkpoint("04-thought-settings"), 5, 2, 170, 235)
        self.tool_tap(5, 3, "lighting")
        self.panel(self.checkpoint("05-brightness"), 5, 3, 235, 300)
        self.tool_tap(5, 4, "typography")
        self.panel(self.checkpoint("06-typography"), 5, 4, 295, 430)
        left, available = self.margin, self.w - 2 * self.margin
        for index, name, limits in (
            (0, "font", (130, 330)), (1, "indent", (145, 305)), (2, "page-turn", (230, 425)),
        ):
            self.tap(left + available * (index + 0.5) / 3, self.h - self.nav - 42, f"{name} subpanel")
            top = self.panel(self.checkpoint(f"07-{index + 1}-{name}-subpanel"), 5, 4, *limits)
            self.tap(left + 38, top + 8 + 25, "return to typography")
        self.tool_tap(5, 0, "Reader Contents leads to book Detail")
        right, y = self.w - self.margin, 176 if self.scribe else 370
        detail = self.checkpoint("08-toolbar-contents-destination",
                                 ready=lambda frame: min(self.pager_ink(frame, right, y)) > 0.01)
        self.pager(detail, right, y)

    def reader_notes(self):
        frame = self.checkpoint("01-cached-popular-thread")
        height = self.h * 62 // 100
        y = (self.h + height) // 2 - 26
        right = self.w // 2 + 76
        self.pager(frame, right, y)
        self.tap(right - 24, y, "next cached discussion page")
        page_two = self.checkpoint("02-cached-popular-thread-page-two")
        self.moved(frame, page_two, (self.w // 4, (self.h - height) // 2 + 160, self.w * 3 // 4, (self.h + height) // 2 - 52))
        self.tap(self.w / 2, y, "discussion jump dialog")
        self.dialog(self.checkpoint("03-popular-thread-jump"))
        self.tap(3, 20, "dismiss jump dialog")
        self.tap(3, 20, "close discussion")
        self.checkpoint("04-return-to-reading")

    def reader_sidebar(self):
        sidebar = self.checkpoint("01-current-page-side-notes")
        x = self.w - 300
        require(ink(sidebar, (x + 1, 74, self.w - 3, 76), maximum=64) > 0.95, "Scribe notes sidebar is not open")
        self.tap(x + 125, 148, "open first cached side note")
        thread = self.checkpoint("02-side-thread")
        self.moved(sidebar, thread, (x, 10, self.w - 8, 420), 0.01)
        self.tap(x + 35, 40, "return to side previews")
        self.tap(self.w - 32, 42, "close side notes")
        closed = self.checkpoint("03-return-to-full-width-reading")
        self.moved(sidebar, closed, (x, 74, self.w - 6, self.h - 64), 0.01)

    def reader_contents(self):
        contents = self.checkpoint("01-retained-reader-contents-list")
        # Unlike the toolbar destination this is the independent, retained Reader overlay.
        right = self.w - self.margin
        self.pager(contents, right, 114)
        self.tap(right - 24, 114, "next Reader contents page")
        second = self.checkpoint("02-contents-page-two")
        self.moved(contents, second, (self.margin, 200, right, self.h - 32))
        tab_y = 174
        self.tap(self.w / 2, tab_y, "bookmarks tab")
        marks = self.checkpoint("03-bookmarks")
        self.moved(contents, marks, (self.margin, 148, right, 360))
        self.pager(marks, right, 114)
        self.tap(self.w * 5 / 6, tab_y, "local notes tab")
        notes = self.checkpoint("04-local-notes")
        self.moved(marks, notes, (self.margin, 148, right, 360))

    def publications(self):
        right = self.w - 8
        body = (self.margin, 129, self.w - self.margin, self.h - self.nav - 32)
        accounts = self.checkpoint("01-accounts")
        self.nav_active(accounts, 1)
        self.pager(accounts, right, 104)
        self.rows(accounts, 129, 13 if self.scribe else 8)
        self.tap(right - 24, 104, "next publications accounts")
        self.moved(accounts, self.checkpoint("02-accounts-page-two"), body)
        self.tap(right - 128, 104, "previous publications accounts")
        self.tap(140, 163, "open the first demo account")
        articles = self.checkpoint("03-account-articles")
        self.moved(accounts, articles, body)
        self.pager(articles, right, 104)
        self.tap(right - 24, 104, "next articles page")
        self.moved(articles, self.checkpoint("04-account-articles-page-two"), body)
        self.tap(32, 104, "back to all accounts")
        self.tap(self.w - (53 if self.scribe else 32), 54, "offline publications sync", delay=0.18)
        self.toast_checkpoint("05-publications-toast", self.nav + 16)
        time.sleep(2)
        self.tap(150, 54, "publications search")
        searching = self.checkpoint("06-publications-search-keyboard")
        top = self.h - 300
        require(ink(searching, (2, top, self.w - 2, top + 2), maximum=64) > 0.95, "Publications search keyboard missing")
        self.tap(self.margin + 24, 56, "back to Publications")

    def article(self):
        reading = self.checkpoint("01-article-and-offline-image-placeholder")
        self.tap(self.w / 2, 100, "open Article menu from the reading header")
        menu = self.checkpoint("02-menu-tools-only")
        self.panel(menu, 4, None, 0, 2)
        self.tap(self.w - 32, 54, "local favourite", delay=0.18)
        self.toast_checkpoint("03-tools-only-toast", self.nav + 16)
        time.sleep(2)
        self.tool_tap(4, 1, "Article Images")
        images = self.checkpoint("04-images-menu")
        top = self.panel(images, 4, 1, 145, 160)
        self.tap(self.w - self.margin - 45, self.h - self.nav - 52, "load all images while offline", delay=0.18)
        self.toast_checkpoint("05-image-load-offline-toast", self.h - top + 16)
        time.sleep(2)
        self.tool_tap(4, 2, "Article typography")
        self.panel(self.checkpoint("06-typography"), 4, 2, 175, 350)
        self.tool_tap(4, 0, "Article drawer")
        drawer = self.checkpoint("07-article-drawer")
        drawer_right = 560 if self.scribe else self.w
        pager_right = drawer_right - 68
        self.pager(drawer, pager_right, 62)
        self.tap(pager_right - 24, 62, "next drawer page")
        following = self.checkpoint("08-article-drawer-page-two")
        self.moved(drawer, following, (self.margin, 98, drawer_right - 10, self.h))
        self.tap(drawer_right - 32, 62, "close drawer")
        self.tool_tap(4, 3, "show synthetic phone link")
        qr = self.checkpoint("09-synthetic-phone-link-qr")
        self.moved(reading, qr, (self.w // 4, self.h // 4, self.w * 3 // 4, self.h * 3 // 4), 0.015)
        self.tap(3, 20, "dismiss article QR")

    def settings(self):
        first = self.checkpoint("01-sync")
        self.nav_active(first, 3)
        footer = 72 if self.scribe else 64
        self.tap(self.w - self.margin - 48, 84 if self.scribe else 76, "offline renew account", delay=0.18)
        self.toast_checkpoint("02-settings-toast", self.nav + footer + 16)
        time.sleep(2)
        previous = first
        for index, tab in enumerate(("reading", "downloads", "display"), 1):
            x = self.margin + (self.w - self.margin * 2) * (index + 0.5) / 4
            self.tap(x, 168 if self.scribe else 150, f"Settings {tab} tab")
            frame = self.checkpoint(f"0{index + 2}-{tab}")
            self.moved(previous, frame, (self.margin, 204 if self.scribe else 184, self.w - self.margin, self.h - self.nav - footer))
            previous = frame
        self.tap(self.w - self.margin - 64, self.h - self.nav - footer / 2, "exit confirmation")
        self.dialog(self.checkpoint("06-exit-confirmation"))
        self.tap(3, 20, "cancel exit outside dialog")

    def stats(self):
        self.tool_tap(4, 2, "Statistics navigation")
        previous = self.checkpoint("01-week")
        self.nav_active(previous, 2)
        span = 380 if self.scribe else self.w - 2 * self.margin
        for index, period in enumerate(("month", "year", "total"), 1):
            self.tap(self.margin + span * (index + 0.5) / 4, 116 if self.scribe else 108, period)
            frame = self.checkpoint(f"0{index + 1}-{period}")
            self.moved(previous, frame, (self.margin, 84, self.w - self.margin, 460))
            previous = frame

    def sleep(self):
        before = self.checkpoint("01-reader-before-sleep")
        # The title/author fallback belongs to the normal cover-idle policy.
        time.sleep(5)
        powerd = "com.lab126.powerd"
        if self.guest("test", "-s", f"{self.lock}/managed-sleep-available", check=False).returncode:
            raise Blocked("Guest did not advertise managed application sleep")
        active = self.guest("lipc-get-prop", "-s", powerd, "state", check=False)
        if active.returncode or active.stdout.strip() != b"active":
            raise Blocked("Guest powerd was not confirmed active before sleep")
        self.guest("lipc-set-prop", "-i", powerd, "powerButton", 1)
        self.inputs.append({"input": "guest powerd", "event": "powerButton=1"})

        def state():
            return self.guest("lipc-get-prop", "-s", powerd, "state", check=False).stdout.strip()

        entered_screensaver = False
        try:
            try:
                self.wait_for(lambda: state().lower() == b"screensaver", 15, "Guest powerd did not enter screenSaver")
                entered_screensaver = True
            except TimeoutError as error:
                policy = {}
                for name in ("preventScreenSaver", "usbState"):
                    result = self.guest("lipc-get-prop", "-i", powerd, name, check=False)
                    if result.returncode == 0:
                        policy[name] = result.stdout.decode(errors="replace").strip()
                self.checkpoint("02-power-state-not-confirmed", power_state=state().decode(errors="replace"),
                                guest_power_policy=policy)
                raise Blocked(str(error)) from error

            def marker(name):
                return self.guest("cat", f"{self.lock}/{name}", check=False).stdout.strip()

            def cover_acknowledged():
                requested = marker("managed-cover-request")
                return bool(requested and requested == marker("managed-cover-ready") == marker("managed-sleep-ready"))

            until = time.monotonic() + 8
            ready = cover_acknowledged()
            while not ready and time.monotonic() < until:
                time.sleep(0.25)
                ready = cover_acknowledged()
            cover = self.checkpoint("02-sleep-cover-scanout", app_cover_and_sleep_acknowledged=ready)
            left, top, right = round(self.w * 0.12), round(self.h * 0.13), round(self.w * 0.88)
            self.moved(before, cover, (self.margin, 80, self.w - self.margin, self.h - 80), 0.01)
            require(any(ink(cover, (left + 4, y, right - 4, y + 1), maximum=64) > 0.98
                        for y in range(top - 2, top + 4)), "The designed title/author fallback frame was not visible")
            if not ready:
                raise Blocked("The application fallback frame was visible but managed-sleep acknowledgement was not confirmed")
        finally:
            if state() != b"active":
                wake = self.guest("lipc-set-prop", "-i", powerd, "wakeUp", 1, check=False)
                self.inputs.append({"input": "guest powerd", "event": "wakeUp=1", "exit_code": wake.returncode})
                if wake.returncode and state() != b"active":
                    self.guest("lipc-set-prop", "-i", powerd, "powerButton", 1, check=False)
                    self.inputs.append({"input": "guest powerd", "event": "powerButton=1 (wake fallback)"})
                self.wait_for(lambda: state() == b"active", 18, "Guest powerd was not active after wake")
            body = (self.margin, 80, self.w - self.margin, self.h - 80)
            checkpoint = "03-reader-after-wake" if entered_screensaver else "03-reader-after-power-request"
            restored = self.checkpoint(checkpoint, ready=lambda frame: difference(before, frame, body) <= 0.001)
        self.unchanged(before, restored, body)

    def login(self):
        offline = self.checkpoint("01-signed-out-offline")
        self.tap(self.w / 2, 1030 if self.scribe else 766, "open Skills guide")
        guide = self.checkpoint("02-built-in-skills-guide")
        self.moved(offline, guide, (self.margin, 52, self.w - self.margin, self.h - 20), 0.012)
        self.tap(self.margin + 48, 68 if self.scribe else 60, "return to Login")
        returned = self.checkpoint("03-return-to-signed-out-login")
        self.moved(guide, returned, (self.margin, 52, self.w - self.margin, self.h - 20), 0.012)
        # Retry updates the status prompt, not the Login form or route.
        self.unchanged(offline, returned, (0, 28, self.w, 125))
