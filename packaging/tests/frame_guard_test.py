"""Exercise the frame guard on an isolated Linux Xvfb display and fake framebuffer.

These tests cover X11 pixel retention and process fencing. They do not verify
Kindle window-manager layers, EPDC waveforms, physical orientation, or firmware.
The 8-bit suite disables Xvfb's backing-store emulation, which quantizes root
readback, and exercises the retained background pixmaps without that path.
Run this file only on the designated remote verification environment.
"""

import array
import ctypes
import hashlib
import json
import os
from pathlib import Path
import select
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest


WIDTH = 1264
HEIGHT = 1680
STRIDE = 1280
TOKEN = "19c084ace627f09231a004167d914bee"
GUARD_NAME_PREFIX = "L:D_N:wereadGuard_"
STOCK_ROOT_COLOR = 0xC85228
SCREEN_SAVER_COLOR = 0x334466
Z_PIXMAP = 2
EXPOSURE_MASK = 1 << 15
PARENT_RELATIVE = 1
CW_BACK_PIXMAP = 1
CW_OVERRIDE_REDIRECT = 1 << 9


def process_identity(pid):
    """Read the actual Linux parent, state, and start time of a fixture child."""
    fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    return fields[0], int(fields[1]), int(fields[19])


def wait_until(predicate, timeout=5.0, description="condition"):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = predicate()
        if result:
            return result
        time.sleep(0.01)
    raise AssertionError(f"Timed out waiting for {description}.")


def write_record(directory, name, value):
    temporary = directory / f"{name}.fixture-pending"
    temporary.write_text(value + "\n")
    temporary.chmod(0o600)
    temporary.replace(directory / name)


def reading_frame(seed, width=WIDTH, height=HEIGHT):
    """Use every gray value, asymmetric edges, and distinct padded scanlines."""
    row = bytes((column * 17 + seed) & 255 for column in range(width))
    return b"".join(
        row.translate(bytes((value + line * 29) & 255 for value in range(256)))
        for line in range(height)
    )


def x_gray_bytes(frame, bits_per_pixel=32):
    """Expected neutral RGB pixels for the explicitly configured Xvfb visual."""
    if bits_per_pixel == 8:
        return frame
    pixels = array.array("I", (value * 0x010101 for value in frame))
    if sys.byteorder != "little":
        pixels.byteswap()
    return pixels.tobytes()


class XImageFunctions(ctypes.Structure):
    _fields_ = [(name, ctypes.c_void_p) for name in (
        "create_image", "destroy_image", "get_pixel", "put_pixel", "sub_image", "add_pixel"
    )]


class XColor(ctypes.Structure):
    _fields_ = [
        ("pixel", ctypes.c_ulong), ("red", ctypes.c_ushort),
        ("green", ctypes.c_ushort), ("blue", ctypes.c_ushort),
        ("flags", ctypes.c_byte), ("pad", ctypes.c_byte),
    ]


class XImage(ctypes.Structure):
    _fields_ = [
        ("width", ctypes.c_int), ("height", ctypes.c_int),
        ("xoffset", ctypes.c_int), ("format", ctypes.c_int),
        ("data", ctypes.c_void_p), ("byte_order", ctypes.c_int),
        ("bitmap_unit", ctypes.c_int), ("bitmap_bit_order", ctypes.c_int),
        ("bitmap_pad", ctypes.c_int), ("depth", ctypes.c_int),
        ("bytes_per_line", ctypes.c_int), ("bits_per_pixel", ctypes.c_int),
        ("red_mask", ctypes.c_ulong), ("green_mask", ctypes.c_ulong),
        ("blue_mask", ctypes.c_ulong), ("obdata", ctypes.c_void_p),
        ("functions", XImageFunctions),
    ]


class XSetWindowAttributes(ctypes.Structure):
    _fields_ = [
        ("background_pixmap", ctypes.c_ulong), ("background_pixel", ctypes.c_ulong),
        ("border_pixmap", ctypes.c_ulong), ("border_pixel", ctypes.c_ulong),
        ("bit_gravity", ctypes.c_int), ("win_gravity", ctypes.c_int),
        ("backing_store", ctypes.c_int), ("backing_planes", ctypes.c_ulong),
        ("backing_pixel", ctypes.c_ulong), ("save_under", ctypes.c_int),
        ("event_mask", ctypes.c_long), ("do_not_propagate_mask", ctypes.c_long),
        ("override_redirect", ctypes.c_int), ("colormap", ctypes.c_ulong),
        ("cursor", ctypes.c_ulong),
    ]


class XExposeEvent(ctypes.Structure):
    _fields_ = [
        ("type", ctypes.c_int), ("serial", ctypes.c_ulong),
        ("send_event", ctypes.c_int), ("display", ctypes.c_void_p),
        ("window", ctypes.c_ulong), ("x", ctypes.c_int), ("y", ctypes.c_int),
        ("width", ctypes.c_int), ("height", ctypes.c_int), ("count", ctypes.c_int),
    ]


class XEvent(ctypes.Union):
    _fields_ = [("type", ctypes.c_int), ("expose", XExposeEvent), ("pad", ctypes.c_long * 24)]


class XDisplay:
    def __init__(self, display_name, width=WIDTH, height=HEIGHT,
            image_layout=(24, 32, 0, 0xFF0000, 0xFF00, 0xFF)):
        self.width = width
        self.height = height
        self.image_layout = image_layout
        self.library = ctypes.CDLL("libX11.so.6")
        pointer = ctypes.c_void_p
        window = ctypes.c_ulong
        unsigned = ctypes.c_uint
        integer = ctypes.c_int
        declarations = {
            "XOpenDisplay": ([ctypes.c_char_p], pointer),
            "XCloseDisplay": ([pointer], integer),
            "XDefaultRootWindow": ([pointer], window),
            "XDefaultScreen": ([pointer], integer),
            "XDefaultColormap": ([pointer, integer], window),
            "XAllocColor": ([pointer, window, ctypes.POINTER(XColor)], integer),
            "XQueryColors": ([pointer, window, ctypes.POINTER(XColor), integer], integer),
            "XCreateSimpleWindow": ([pointer, window, integer, integer, unsigned,
                unsigned, unsigned, window, window], window),
            "XCreateWindow": ([pointer, window, integer, integer, unsigned, unsigned,
                unsigned, integer, unsigned, pointer, window,
                ctypes.POINTER(XSetWindowAttributes)], window),
            "XDestroyWindow": ([pointer, window], integer),
            "XMapRaised": ([pointer, window], integer),
            "XUnmapWindow": ([pointer, window], integer),
            "XStoreName": ([pointer, window, ctypes.c_char_p], integer),
            "XFetchName": ([pointer, window, ctypes.POINTER(pointer)], integer),
            "XQueryTree": ([pointer, window, ctypes.POINTER(window),
                ctypes.POINTER(window), ctypes.POINTER(ctypes.POINTER(window)),
                ctypes.POINTER(unsigned)], integer),
            "XSetWindowBackground": ([pointer, window, window], integer),
            "XClearWindow": ([pointer, window], integer),
            "XCreateGC": ([pointer, window, window, pointer], pointer),
            "XFreeGC": ([pointer, pointer], integer),
            "XSetForeground": ([pointer, pointer, window], integer),
            "XFillRectangle": ([pointer, window, pointer, integer, integer,
                unsigned, unsigned], integer),
            "XGetImage": ([pointer, window, integer, integer, unsigned, unsigned,
                window, integer], ctypes.POINTER(XImage)),
            "XDestroyImage": ([ctypes.POINTER(XImage)], integer),
            "XSendEvent": ([pointer, window, integer, ctypes.c_long,
                ctypes.POINTER(XEvent)], integer),
            "XSync": ([pointer, integer], integer),
            "XFree": ([pointer], integer),
        }
        for name, (arguments, result) in declarations.items():
            function = getattr(self.library, name)
            function.argtypes = arguments
            function.restype = result
        self.display = self.library.XOpenDisplay(display_name.encode())
        if not self.display:
            raise RuntimeError("The isolated Xvfb display could not be opened.")
        self.root = self.library.XDefaultRootWindow(self.display)
        self.gc = self.library.XCreateGC(self.display, self.root, 0, None)
        root_color = STOCK_ROOT_COLOR if image_layout[1] == 32 else STOCK_ROOT_COLOR & 255
        self.library.XSetWindowBackground(self.display, self.root, root_color)
        self.sync()

    def sync(self):
        self.library.XSync(self.display, 0)

    def close(self):
        self.library.XFreeGC(self.display, self.gc)
        self.library.XCloseDisplay(self.display)

    def create_window(self, name, color, override_root_background=False):
        if override_root_background:
            attributes = XSetWindowAttributes()
            attributes.background_pixmap = PARENT_RELATIVE
            attributes.override_redirect = 1
            window = self.library.XCreateWindow(self.display, self.root, 0, 0,
                self.width, self.height, 0, 0, 1, None,
                CW_BACK_PIXMAP | CW_OVERRIDE_REDIRECT, ctypes.byref(attributes))
        else:
            window = self.library.XCreateSimpleWindow(self.display, self.root,
                0, 0, self.width, self.height, 0, 0, color)
        self.library.XStoreName(self.display, window, name.encode())
        self.library.XMapRaised(self.display, window)
        self.sync()
        return window

    def destroy_window(self, window):
        self.library.XDestroyWindow(self.display, window)
        self.sync()

    def fill(self, window, color, x=0, y=0, width=None, height=None):
        width = self.width if width is None else width
        height = self.height if height is None else height
        self.library.XSetForeground(self.display, self.gc, color)
        self.library.XFillRectangle(self.display, window, self.gc, x, y, width, height)
        self.sync()

    def guard_windows(self):
        root = ctypes.c_ulong()
        parent = ctypes.c_ulong()
        children = ctypes.POINTER(ctypes.c_ulong)()
        count = ctypes.c_uint()
        if not self.library.XQueryTree(self.display, self.root, ctypes.byref(root),
                ctypes.byref(parent), ctypes.byref(children), ctypes.byref(count)):
            raise AssertionError("The isolated X11 window tree could not be read.")
        result = []
        try:
            for index in range(count.value):
                name = ctypes.c_void_p()
                window = children[index]
                if self.library.XFetchName(self.display, window, ctypes.byref(name)) and name.value:
                    try:
                        title = ctypes.string_at(name.value).decode("utf-8", errors="replace")
                        if title.startswith(GUARD_NAME_PREFIX):
                            result.append(window)
                    finally:
                        self.library.XFree(name)
        finally:
            if children:
                self.library.XFree(children)
        return result

    def pixels(self):
        image = self.library.XGetImage(self.display, self.root, 0, 0,
            self.width, self.height, ctypes.c_ulong(-1).value, Z_PIXMAP)
        if not image:
            raise AssertionError("The isolated X11 frame could not be read.")
        try:
            layout = image.contents
            if (layout.depth, layout.bits_per_pixel, layout.byte_order,
                    layout.red_mask, layout.green_mask, layout.blue_mask) != self.image_layout:
                raise AssertionError("The Xvfb test visual differs from the requested fixture.")
            row_bytes = self.width * layout.bits_per_pixel // 8
            data = ctypes.string_at(layout.data, layout.bytes_per_line * self.height)
            if layout.bytes_per_line == row_bytes:
                return data
            return b"".join(data[row * layout.bytes_per_line:row * layout.bytes_per_line + row_bytes]
                for row in range(self.height))
        finally:
            self.library.XDestroyImage(image)

    def gray_palette(self):
        """Query existing cells independently of the helper's color allocation."""
        colormap = self.library.XDefaultColormap(
            self.display, self.library.XDefaultScreen(self.display))
        colors = (XColor * 256)()
        for value, color in enumerate(colors):
            color.pixel = value
        if not self.library.XQueryColors(self.display, colormap, colors, len(colors)):
            raise AssertionError("The isolated X11 palette could not be queried.")
        return [(color.red, color.green, color.blue) for color in colors]

    def allocate_gray(self, value):
        colormap = self.library.XDefaultColormap(
            self.display, self.library.XDefaultScreen(self.display))
        color = XColor(red=value * 257, green=value * 257, blue=value * 257, flags=7)
        if not self.library.XAllocColor(self.display, colormap, ctypes.byref(color)):
            raise AssertionError("The isolated X11 gray could not be allocated.")
        return color.pixel, (color.red, color.green, color.blue)

    def send_expose(self, window):
        event = XEvent()
        event.expose.type = 12
        event.expose.send_event = 1
        event.expose.display = self.display
        event.expose.window = window
        event.expose.width = self.width
        event.expose.height = self.height
        if not self.library.XSendEvent(self.display, window, 0, EXPOSURE_MASK,
                ctypes.byref(event)):
            raise AssertionError("The synthetic Expose event was not submitted.")
        self.sync()


class GuardFixture:
    def __init__(self, test, frozen=True, metadata=None, startup=True):
        self.test = test
        self.directory = test.root / f"guard-{len(test.fixtures)}"
        self.directory.mkdir(mode=0o700)
        self.context = self.directory / "context"
        self.context.mkdir(mode=0o700)
        self.framebuffer = self.directory / "framebuffer.raw"
        self.metadata_file = self.directory / "framebuffer.metadata"
        self.metadata = metadata or f"{test.width}:{test.height}:{test.stride}:1"
        self.set_metadata(self.metadata)
        self.write_frame(test.first_frame)
        self.owner = os.getpid()
        self.owner_birth = process_identity(self.owner)[2]
        write_record(self.context, "pid", str(self.owner))
        write_record(self.context, "nonce", TOKEN)
        write_record(self.context, "generation", f"1:{TOKEN}")
        self.application = subprocess.Popen(["sleep", "60"], close_fds=True)
        _, self.application_parent, self.application_birth = process_identity(self.application.pid)
        if frozen:
            self.application.send_signal(signal.SIGSTOP)
            wait_until(lambda: process_identity(self.application.pid)[0] in ("T", "t"),
                description="the fixture application's stopped state")
        write_record(self.context, "application-pid", str(self.application.pid))
        write_record(self.context, "application-birth", str(self.application_birth))
        self.read_fd, self.write_fd = os.pipe()
        self.stderr_path = self.directory / "helper.stderr"
        self.stderr_file = self.stderr_path.open("wb")
        environment = test.environment.copy()
        environment.update(
            WEREAD_TEST_FRAMEBUFFER=str(self.framebuffer),
            WEREAD_TEST_FRAMEBUFFER_METADATA=self.metadata,
            WEREAD_TEST_FRAMEBUFFER_METADATA_FILE=str(self.metadata_file),
        )
        self.process = subprocess.Popen([
            str(test.helper), "--context", str(self.context), "--token", TOKEN,
            "--owner", str(self.owner), "--owner-birth", str(self.owner_birth),
            "--profile", test.profile, "--command-fd", str(self.read_fd),
        ], env=environment, pass_fds=(self.read_fd,), stdout=subprocess.DEVNULL,
            stderr=self.stderr_file)
        os.close(self.read_fd)
        self.read_fd = None
        test.fixtures.append(self)
        self.ready_path = self.context / "frame-guard-ready"
        self.helper_birth = process_identity(self.process.pid)[2]
        if startup:
            self.wait_ready(0, 0)
            windows = test.display.guard_windows()
            test.assertEqual(len(windows), 1, "The guard must use only the isolated Xvfb display.")
            self.window = windows[0]
            window_record = self.context / "frame-guard-window"
            test.assertEqual(window_record.read_text(),
                f"{TOKEN}:{self.process.pid}:{self.helper_birth}:0x{self.window:x}\n")
            test.assertEqual(window_record.stat().st_mode & 0o777, 0o600)

    def set_metadata(self, value):
        self.metadata_file.write_text(value + "\n")
        self.metadata_file.chmod(0o600)

    def write_frame(self, frame, padding=0x93):
        width, height, stride = self.test.width, self.test.height, self.test.stride
        padding = bytes([padding]) * (stride - width)
        with self.framebuffer.open("wb") as output:
            for row in range(height):
                output.write(frame[row * width:(row + 1) * width])
                output.write(padding)
        self.framebuffer.chmod(0o600)

    def expected_ready(self, sequence, generation):
        return f"{sequence}:{generation}:{TOKEN}:{self.process.pid}:{self.helper_birth}\n"

    def wait_ready(self, sequence, generation=1):
        expected = self.expected_ready(sequence, generation)
        def matching_record():
            if self.process.poll() is not None:
                raise AssertionError(f"The helper exited before acknowledgment: {self.stderr_path.read_text()}")
            try:
                return self.ready_path.read_text() == expected
            except FileNotFoundError:
                return False
        wait_until(matching_record, description=f"frame guard acknowledgment {sequence}")
        self.test.assertEqual(self.ready_path.stat().st_mode & 0o777, 0o600)

    def request(self, sequence, operation="capture", token=TOKEN, generation=1,
            birth=None, parent=None):
        record = (f"{sequence}:{generation}:{token}:{self.application.pid}:"
            f"{self.application_birth if birth is None else birth}:"
            f"{self.application_parent if parent is None else parent}:{operation}\n")
        os.write(self.write_fd, record.encode())

    def expect_rejected(self, previous_sequence=0, previous_generation=0):
        self.test.assertEqual(self.process.wait(timeout=6), 1)
        self.test.assertEqual(self.ready_path.read_text(),
            self.expected_ready(previous_sequence, previous_generation))
        wait_until(lambda: not self.test.display.guard_windows(),
            description="rejected guard window cleanup")

    def close(self):
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=6)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=2)
        if self.write_fd is not None:
            os.close(self.write_fd)
            self.write_fd = None
        if self.application.poll() is None:
            self.application.send_signal(signal.SIGCONT)
            self.application.terminate()
            try:
                self.application.wait(timeout=2)
            except subprocess.TimeoutExpired:
                self.application.kill()
                self.application.wait(timeout=2)
        self.stderr_file.close()


def fixture_owner(helper, context, read_fd):
    """Leave a live writer in the test parent to distinguish death from pipe EOF."""
    context = Path(context)
    owner = os.getpid()
    write_record(context, "pid", str(owner))
    write_record(context, "nonce", TOKEN)
    subprocess.Popen([str(helper), "--context", str(context), "--token", TOKEN,
        "--owner", str(owner), "--owner-birth", str(process_identity(owner)[2]),
        "--profile", "pw6", "--command-fd", str(read_fd)],
        pass_fds=(read_fd,), stdout=subprocess.DEVNULL)
    while True:
        signal.pause()


class _FrameGuardFixture(unittest.TestCase):
    width = WIDTH
    height = HEIGHT
    stride = STRIDE
    profile = "pw6"
    visual_class = 4
    image_layout = (24, 32, 0, 0xFF0000, 0xFF00, 0xFF)
    xvfb_options = ()

    @classmethod
    def setUpClass(cls):
        if sys.platform != "linux":
            raise RuntimeError("Run frame guard verification only on the designated Linux test environment.")
        for tool in ("Xvfb", "xrefresh"):
            if not shutil.which(tool):
                raise unittest.SkipTest(f"The remote X11 fixture requires {tool}.")
        cls.temporary = tempfile.TemporaryDirectory(prefix="weread-frame-guard-test-")
        cls.addClassCleanup(cls.temporary.cleanup)
        cls.suite_root = Path(cls.temporary.name)
        supplied_helper = os.environ.get("WEREAD_FRAME_GUARD_TEST_BINARY")
        if supplied_helper:
            cls.helper = Path(supplied_helper).resolve()
        else:
            if not shutil.which("cc"):
                raise unittest.SkipTest("The remote C fixture requires cc.")
            source = Path(__file__).resolve().parents[1] / "kindle" / "frame_guard.c"
            cls.helper = cls.suite_root / "frame-guard-test"
            subprocess.run(["cc", "-std=gnu11", "-O2", "-Wall", "-Wextra", "-Werror",
                "-DWEREAD_FRAME_GUARD_TEST", str(source), "-ldl", "-o", str(cls.helper)],
                check=True, capture_output=True, text=True, timeout=30)
        if not cls.helper.is_file():
            raise RuntimeError("The requested test helper is unavailable.")
        cls.xvfb_log = (cls.suite_root / "xvfb.stderr").open("wb")
        cls.addClassCleanup(cls.xvfb_log.close)
        read_fd, write_fd = os.pipe()
        cls.xvfb = subprocess.Popen(["Xvfb", "-displayfd", str(write_fd), "-screen", "0",
            f"{cls.width}x{cls.height}x{cls.image_layout[0]}", "-cc", str(cls.visual_class),
            "-nolisten", "tcp", "-ac", *cls.xvfb_options],
            pass_fds=(write_fd,), stdout=subprocess.DEVNULL, stderr=cls.xvfb_log)
        os.close(write_fd)
        cls.addClassCleanup(cls.stop_xvfb)
        try:
            readable, _, _ = select.select([read_fd], [], [], 5)
            if not readable:
                raise RuntimeError("Xvfb did not publish an isolated display number.")
            display_number = os.read(read_fd, 32).decode().strip()
            if not display_number.isdecimal():
                raise RuntimeError("Xvfb published an invalid display number.")
        finally:
            os.close(read_fd)
        cls.environment = os.environ.copy()
        cls.environment["DISPLAY"] = f":{display_number}"
        cls.display = XDisplay(cls.environment["DISPLAY"], cls.width, cls.height, cls.image_layout)
        cls.addClassCleanup(cls.display.close)
        cls.first_frame = reading_frame(23, cls.width, cls.height)
        cls.second_frame = reading_frame(118, cls.width, cls.height)
        cls.first_pixels = x_gray_bytes(cls.first_frame, cls.image_layout[1])
        cls.second_pixels = x_gray_bytes(cls.second_frame, cls.image_layout[1])
        artifact_directory = os.environ.get("WEREAD_FRAME_GUARD_TEST_ARTIFACT_DIR")
        cls.artifacts = Path(artifact_directory).resolve() if artifact_directory else None
        if cls.artifacts:
            cls.artifacts.mkdir(parents=True, exist_ok=True)

    @classmethod
    def stop_xvfb(cls):
        if cls.xvfb.poll() is None:
            cls.xvfb.terminate()
            try:
                cls.xvfb.wait(timeout=3)
            except subprocess.TimeoutExpired:
                cls.xvfb.kill()
                cls.xvfb.wait(timeout=2)

    def setUp(self):
        self.root = self.suite_root / self._testMethodName
        self.root.mkdir(mode=0o700)
        self.fixtures = []
        self.native = self.display.create_window("synthetic-native-library", 0xA62944)
        # Distinct native quadrants expose any accidental stock pixel reuse.
        self.display.fill(self.native, 0x187A38, self.width // 2, 0, self.width // 2, self.height // 2)
        self.display.fill(self.native, 0x225AC4, 0, self.height // 2, self.width // 2, self.height // 2)
        self.display.fill(self.native, 0x912CB3, self.width // 2, self.height // 2,
            self.width // 2, self.height // 2)

    def tearDown(self):
        for fixture in reversed(self.fixtures):
            fixture.close()
        wait_until(lambda: not self.display.guard_windows(), description="fixture guard cleanup")
        self.display.destroy_window(self.native)

    def assert_frame(self, expected):
        actual = self.display.pixels()
        self.assertTrue(actual == expected,
            "The visible frame differs: "
            f"actual={hashlib.sha256(actual).hexdigest()}, "
            f"expected={hashlib.sha256(expected).hexdigest()}.")

    def save_artifact(self, name):
        if not self.artifacts:
            return
        pixels = self.display.pixels()
        rgb = bytearray(self.width * self.height * 3)
        if self.image_layout[1] == 8:
            rgb[0::3] = rgb[1::3] = rgb[2::3] = pixels
        else:
            rgb[0::3] = pixels[2::4]
            rgb[1::3] = pixels[1::4]
            rgb[2::3] = pixels[0::4]
        (self.artifacts / f"{name}.ppm").write_bytes(
            f"P6\n{self.width} {self.height}\n255\n".encode() + rgb)


class FrameGuardTests(_FrameGuardFixture):

    def test_bootstrap_is_blank_and_capture_ignores_stride_padding(self):
        fixture = GuardFixture(self)
        self.assert_frame(bytes([255, 255, 255, 0]) * (WIDTH * HEIGHT))
        fixture.request(1)
        fixture.wait_ready(1)
        self.assert_frame(self.first_pixels)
        self.assertEqual(process_identity(fixture.application.pid)[0], "T")
        self.assertEqual(len(self.display.guard_windows()), 1)
        for record in fixture.context.iterdir():
            self.assertLess(record.stat().st_size, 256, "Pixel data must not enter context records.")
        self.save_artifact("retained-reading-frame")

    def test_no_background_xrefresh_preserves_the_retained_reading_frame(self):
        fixture = GuardFixture(self)
        fixture.request(1)
        fixture.wait_ready(1)
        subprocess.run(["xrefresh", "-none"], env=self.environment,
            check=True, capture_output=True, timeout=5)
        self.display.sync()
        self.assert_frame(self.first_pixels)
        self.save_artifact("reading-after-xrefresh-none")

    def test_root_background_override_emulation_records_a_known_limit(self):
        fixture = GuardFixture(self)
        fixture.request(1)
        fixture.wait_ready(1)
        # A temporary override window can cover even an opaque managed guard.
        # This emulates -root background selection, not Kindle EPDC behavior.
        temporary = self.display.create_window("synthetic-xrefresh-root-override", 0,
            override_root_background=True)
        try:
            root_pixels = array.array("I", [STOCK_ROOT_COLOR]) * (WIDTH * HEIGHT)
            if sys.byteorder != "little":
                root_pixels.byteswap()
            self.assert_frame(root_pixels.tobytes())
            self.save_artifact("root-background-override-limit")
            self.display.library.XUnmapWindow(self.display.display, temporary)
            self.display.sync()
            self.assert_frame(self.first_pixels)
        finally:
            self.display.destroy_window(temporary)

    def test_screen_saver_stacking_reveals_reading_after_dismissal(self):
        fixture = GuardFixture(self)
        fixture.request(1)
        fixture.wait_ready(1)
        # Xvfb has no Kindle layer policy; this models only higher X11 stacking.
        screen_saver = self.display.create_window(
            "L:SS_N:screenSaver_ID:synthetic-screen-saver_O:UD", SCREEN_SAVER_COLOR)
        try:
            screen_pixels = array.array("I", [SCREEN_SAVER_COLOR]) * (WIDTH * HEIGHT)
            if sys.byteorder != "little":
                screen_pixels.byteswap()
            self.assert_frame(screen_pixels.tobytes())
            self.display.library.XUnmapWindow(self.display.display, screen_saver)
            self.display.sync()
            self.assert_frame(self.first_pixels)
            self.save_artifact("reading-after-screen-saver-dismissal")
        finally:
            self.display.destroy_window(screen_saver)

    def test_capture_does_not_acknowledge_an_obscured_guard(self):
        fixture = GuardFixture(self)
        screen_saver = self.display.create_window(
            "L:SS_N:screenSaver_ID:synthetic-screen-saver_O:UD", SCREEN_SAVER_COLOR)
        try:
            fixture.request(1)
            fixture.expect_rejected()
            screen_pixels = array.array("I", [SCREEN_SAVER_COLOR]) * (WIDTH * HEIGHT)
            if sys.byteorder != "little":
                screen_pixels.byteswap()
            self.assert_frame(screen_pixels.tobytes())
        finally:
            self.display.destroy_window(screen_saver)

    def test_idle_expose_does_not_redraw_an_old_application_frame(self):
        fixture = GuardFixture(self)
        fixture.request(1)
        fixture.wait_ready(1)
        replacement_color = 0x2277CC
        self.display.fill(fixture.window, replacement_color)
        for _ in range(8):
            self.display.send_expose(fixture.window)
        time.sleep(0.15)
        replacement = array.array("I", [replacement_color]) * (WIDTH * HEIGHT)
        if sys.byteorder != "little":
            replacement.byteswap()
        self.assert_frame(replacement.tobytes())
        self.assertIsNone(fixture.process.poll())

    def test_capture_replaces_the_pixmap_without_partial_visible_frames(self):
        fixture = GuardFixture(self)
        fixture.request(1)
        fixture.wait_ready(1)
        previous = self.first_pixels
        for sequence, frame, expected in ((2, self.second_frame, self.second_pixels),
                (3, self.first_frame, self.first_pixels), (4, self.second_frame, self.second_pixels)):
            fixture.write_frame(frame)
            fixture.request(sequence)
            for _ in range(4):
                actual = self.display.pixels()
                self.assertTrue(actual == previous or actual == expected,
                    "A pixmap replacement exposed stock pixels or a partially copied frame.")
            fixture.wait_ready(sequence)
            self.assert_frame(expected)
            self.assertEqual(len(self.display.guard_windows()), 1)
            previous = expected

    def test_align_uses_only_relative_portrait_half_turns(self):
        fixture = GuardFixture(self)
        fixture.request(1)
        fixture.wait_ready(1)
        fixture.set_metadata(f"{WIDTH}:{HEIGHT}:{STRIDE}:3")
        fixture.request(2, operation="align")
        fixture.wait_ready(2)
        self.assert_frame(x_gray_bytes(self.first_frame[::-1]))
        fixture.set_metadata(f"{WIDTH}:{HEIGHT}:{STRIDE}:1")
        fixture.request(3, operation="align")
        fixture.wait_ready(3)
        self.assert_frame(self.first_pixels)

    def test_align_rejects_a_relative_quarter_turn(self):
        fixture = GuardFixture(self)
        fixture.request(1)
        fixture.wait_ready(1)
        fixture.set_metadata(f"{WIDTH}:{HEIGHT}:{STRIDE}:0")
        fixture.request(2, operation="align")
        fixture.expect_rejected(1, 1)

    def test_capture_rejects_a_changed_framebuffer_size(self):
        fixture = GuardFixture(self)
        fixture.request(1)
        fixture.wait_ready(1)
        fixture.set_metadata(f"{WIDTH - 1}:{HEIGHT}:{STRIDE}:1")
        fixture.request(2)
        fixture.expect_rejected(1, 1)

    def test_capture_rejects_a_truncated_framebuffer(self):
        fixture = GuardFixture(self)
        fixture.framebuffer.write_bytes(b"truncated")
        fixture.request(1)
        fixture.expect_rejected()

    def test_rejects_a_stale_sequence_without_republishing_ready(self):
        fixture = GuardFixture(self)
        fixture.request(1)
        fixture.wait_ready(1)
        fixture.request(1)
        fixture.expect_rejected(1, 1)

    def test_rejects_a_foreign_token(self):
        fixture = GuardFixture(self)
        fixture.request(1, token="a" * 32)
        fixture.expect_rejected()

    def test_rejects_a_running_application(self):
        fixture = GuardFixture(self, frozen=False)
        self.assertNotIn(process_identity(fixture.application.pid)[0], ("T", "t"))
        fixture.request(1)
        fixture.expect_rejected()

    def test_rejects_a_replaced_application_birth(self):
        fixture = GuardFixture(self)
        fixture.request(1, birth=fixture.application_birth + 1)
        fixture.expect_rejected()

    def test_rejects_a_replaced_application_parent(self):
        fixture = GuardFixture(self)
        fixture.request(1, parent=fixture.application_parent + 1)
        fixture.expect_rejected()

    def test_rejects_a_stale_generation(self):
        fixture = GuardFixture(self)
        write_record(fixture.context, "generation", f"2:{TOKEN}")
        fixture.request(1)
        fixture.expect_rejected()

    def test_rejects_an_oversized_command_record(self):
        fixture = GuardFixture(self)
        os.write(fixture.write_fd, b"x" * 300 + b"\n")
        fixture.expect_rejected()

    def test_parent_death_removes_the_window_with_a_live_command_writer(self):
        context = self.root / "owner-context"
        context.mkdir(mode=0o700)
        framebuffer = self.root / "owner-framebuffer.raw"
        framebuffer.write_bytes(bytes(STRIDE * HEIGHT))
        framebuffer.chmod(0o600)
        read_fd, write_fd = os.pipe()
        environment = self.environment.copy()
        environment.update(WEREAD_TEST_FRAMEBUFFER=str(framebuffer),
            WEREAD_TEST_FRAMEBUFFER_METADATA=f"{WIDTH}:{HEIGHT}:{STRIDE}:1")
        environment.pop("WEREAD_TEST_FRAMEBUFFER_METADATA_FILE", None)
        stderr = (self.root / "owner-helper.stderr").open("wb")
        owner = subprocess.Popen([sys.executable, str(Path(__file__).resolve()),
            "--fixture-owner", str(self.helper), str(context), str(read_fd)],
            env=environment, pass_fds=(read_fd,), stdout=subprocess.DEVNULL, stderr=stderr)
        os.close(read_fd)
        try:
            ready = context / "frame-guard-ready"
            wait_until(ready.exists, description="the dedicated owner's guard startup")
            record = ready.read_text().strip().split(":")
            self.assertEqual(record[:3], ["0", "0", TOKEN])
            self.assertEqual(len(self.display.guard_windows()), 1)
            helper_pid = int(record[3])
            self.assertEqual(process_identity(helper_pid)[1], owner.pid)
            owner.kill()
            owner.wait(timeout=3)
            # The writer stays open here, so pipe EOF cannot explain cleanup.
            wait_until(lambda: not self.display.guard_windows(),
                description="the dead owner's guard window removal")
            self.assertEqual(os.fstat(write_fd).st_mode & 0o170000, 0o010000)
        finally:
            if owner.poll() is None:
                owner.kill()
                owner.wait(timeout=3)
            os.close(write_fd)
            stderr.close()


class ScribeStaticGrayTests(_FrameGuardFixture):
    width = 1860
    height = 2480
    stride = 1872
    profile = "scribe"
    visual_class = 0
    image_layout = (8, 8, 0, 0, 0, 0)
    xvfb_options = ("-bs",)

    def native_probe(self):
        result = subprocess.run([str(self.helper), "--probe-native-x11"],
            env=self.environment, check=True, capture_output=True, text=True, timeout=6)
        return json.loads(result.stdout)

    def test_static_gray_palette_and_probe_match_the_scribe_layout(self):
        probe = self.native_probe()
        expected = {
            "width": self.width, "height": self.height, "depth": 8,
            "visual_class": 0, "red_mask": 0, "green_mask": 0, "blue_mask": 0,
            "bits_per_pixel": 8, "bytes_per_line": 1860, "byte_order": 0,
            "gray_map_exact": 1, "gray_map_identity": 1,
        }
        for field, value in expected.items():
            self.assertEqual(probe[field], value, f"Unexpected StaticGray probe field: {field}.")
        palette = self.display.gray_palette()
        self.assertEqual(palette, [(value * 257,) * 3 for value in range(256)])
        for value in range(256):
            self.assertEqual(self.display.allocate_gray(value), (value, (value * 257,) * 3))

    def test_all_grays_ignore_framebuffer_padding_and_remain_retained(self):
        fixture = GuardFixture(self)
        self.assert_frame(bytes([255]) * (self.width * self.height))
        self.assertEqual(set(self.first_frame), set(range(256)))
        fixture.write_frame(self.first_frame, padding=0xE7)
        fixture.request(1)
        fixture.wait_ready(1)
        self.assert_frame(self.first_frame)
        fixture.write_frame(self.second_frame, padding=0x4A)
        self.display.send_expose(fixture.window)
        subprocess.run(["xrefresh", "-none"], env=self.environment,
            check=True, capture_output=True, timeout=5)
        self.display.sync()
        self.assert_frame(self.first_frame)
        self.assertEqual(self.display.guard_windows(), [fixture.window])
        self.save_artifact("scribe-static-gray-retained-reading-frame")

    def test_capture_replaces_complete_static_gray_frames(self):
        fixture = GuardFixture(self)
        fixture.request(1)
        fixture.wait_ready(1)
        previous = self.first_frame
        for sequence, frame in ((2, self.second_frame), (3, self.first_frame),
                (4, self.second_frame)):
            fixture.write_frame(frame)
            fixture.request(sequence)
            for _ in range(4):
                actual = self.display.pixels()
                self.assertTrue(actual == previous or actual == frame,
                    "A StaticGray replacement exposed stock pixels or a partial frame.")
            fixture.wait_ready(sequence)
            self.assert_frame(frame)
            self.assertEqual(self.display.guard_windows(), [fixture.window])
            previous = frame

    def test_relative_half_turn_uses_the_retained_static_gray_snapshot(self):
        fixture = GuardFixture(self)
        fixture.request(1)
        fixture.wait_ready(1)
        fixture.write_frame(self.second_frame)
        fixture.set_metadata(f"{self.width}:{self.height}:{self.stride}:3")
        fixture.request(2, operation="align")
        fixture.wait_ready(2)
        self.assert_frame(self.first_frame[::-1])
        fixture.set_metadata(f"{self.width}:{self.height}:{self.stride}:1")
        fixture.request(3, operation="align")
        fixture.wait_ready(3)
        self.assert_frame(self.first_frame)
        self.assertEqual(self.display.guard_windows(), [fixture.window])

    def test_screen_saver_dismissal_reveals_the_retained_static_gray_frame(self):
        fixture = GuardFixture(self)
        fixture.request(1)
        fixture.wait_ready(1)
        screen_color = 0x66
        screen_saver = self.display.create_window(
            "L:SS_N:screenSaver_ID:synthetic-screen-saver_O:UD", screen_color)
        try:
            self.assert_frame(bytes([screen_color]) * (self.width * self.height))
            fixture.write_frame(self.second_frame)
            self.display.library.XUnmapWindow(self.display.display, screen_saver)
            self.display.sync()
            self.assert_frame(self.first_frame)
            self.assertEqual(self.display.guard_windows(), [fixture.window])
            self.save_artifact("scribe-static-gray-after-screen-saver-dismissal")
        finally:
            self.display.destroy_window(screen_saver)

    def test_request_bands_preserve_all_rows_and_the_final_partial_band(self):
        probe = self.native_probe()
        self.assertGreaterEqual(probe["max_request_words"], 64)
        band_rows = min(64, (probe["max_request_words"] * 4 - 64) // probe["bytes_per_line"])
        # This covers the standard Xvfb limit, not a reduced request-limit branch.
        self.assertEqual(band_rows, 64)
        self.assertLessEqual(band_rows * probe["bytes_per_line"] + 64,
            probe["max_request_words"] * 4)
        self.assertEqual((self.height + band_rows - 1) // band_rows, 39)
        self.assertEqual(self.height % band_rows, 48)
        frame = bytearray(self.first_frame)
        for row in range(self.height):
            frame[row * self.width] = row & 255
            frame[(row + 1) * self.width - 1] = row // band_rows
        fixture = GuardFixture(self)
        fixture.write_frame(frame, padding=0xFA)
        fixture.request(1)
        fixture.wait_ready(1)
        self.assert_frame(frame)
        self.assertEqual(self.display.guard_windows(), [fixture.window])


def main():
    if len(sys.argv) == 5 and sys.argv[1] == "--fixture-owner":
        fixture_owner(Path(sys.argv[2]), Path(sys.argv[3]), int(sys.argv[4]))
        return
    program = unittest.main(verbosity=2, exit=False)
    artifacts = os.environ.get("WEREAD_FRAME_GUARD_TEST_ARTIFACT_DIR")
    if artifacts:
        result = program.result
        helper = os.environ.get("WEREAD_FRAME_GUARD_TEST_BINARY")
        summary = {
            "tests_run": result.testsRun,
            "failures": len(result.failures),
            "errors": len(result.errors),
            "skipped": len(result.skipped),
            "environment": "isolated Xvfb with a synthetic regular-file framebuffer",
            "not_verified": ["Kindle layer and focus policy", "EPDC inversion and waveforms",
                "physical orientation", "firmware or device compatibility"],
            "root_background_override": "explicitly demonstrated coverage limit",
            "static_gray_fixture": {
                "profile": "scribe", "width": 1860, "height": 2480,
                "framebuffer_stride": 1872, "ximage_bytes_per_line": 1860,
                "depth": 8, "visual_class": 0, "bits_per_pixel": 8,
                "xvfb_backing_store": "disabled (-bs) to avoid root-readback quantization",
                "request_bands": "standard Xvfb limit: 64 rows, final band 48 rows",
            },
        }
        if helper:
            summary["helper_sha256"] = hashlib.sha256(Path(helper).read_bytes()).hexdigest()
        Path(artifacts).mkdir(parents=True, exist_ok=True)
        (Path(artifacts) / "result.json").write_text(json.dumps(summary, indent=2) + "\n")
    raise SystemExit(0 if program.result.wasSuccessful() else 1)


if __name__ == "__main__":
    main()
