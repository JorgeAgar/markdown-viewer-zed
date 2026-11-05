#!/usr/bin/env python3
"""Run actual native renderer/startup checks; never replace GUI failures with skips.

No product hooks or third-party Python packages. Artifacts include startup logs,
light/dark desktop screenshots and a machine-readable report for review.
"""
import argparse
import json
import os
from pathlib import Path
import statistics
import subprocess
import sys
import time


def checked(command, **kwargs):
    return subprocess.run(command, check=True, timeout=60, **kwargs)


def notify_windows_theme_change():
    import ctypes
    send = ctypes.windll.user32.SendMessageTimeoutW
    send.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_size_t, ctypes.c_void_p,
                     ctypes.c_uint, ctypes.c_uint, ctypes.POINTER(ctypes.c_size_t)]
    send.restype = ctypes.c_void_p
    result = ctypes.c_size_t()
    setting = ctypes.c_wchar_p("ImmersiveColorSet")
    if not send(0xFFFF, 0x001A, 0, ctypes.cast(setting, ctypes.c_void_p), 2, 5000, ctypes.byref(result)):
        raise ctypes.WinError()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    parser.add_argument("--output", type=Path, default=Path("target/native-validation"))
    args = parser.parse_args()
    binary = args.binary.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    report = {"platform": sys.platform, "binary": str(binary), "first_frame_ms": []}
    (output / "report.json").write_text(json.dumps(report, indent=2))
    demo = Path("examples/demo.md").resolve()
    env = {**os.environ, "MDVIEW_TRACE_STARTUP": "quit"}
    # Five optimized launches measure real presentation, not headless layout.
    for attempt in range(5):
        result = subprocess.run([str(binary), "-n", str(demo)], env=env,
                                capture_output=True, encoding="utf-8", timeout=60)
        (output / f"startup-{attempt}.log").write_text(result.stdout + result.stderr)
        if result.returncode:
            sys.stderr.write(result.stderr)
            result.check_returncode()
        line = next((line.split("first frame: ", 1)[1] for line in result.stderr.splitlines()
                     if line.startswith("first frame: ")), None)
        if line is None:
            raise RuntimeError("native renderer did not report a presented first frame")
        for unit, multiplier in [("ms", 1), ("µs", 0.001), ("ns", 0.000001), ("s", 1000)]:
            if line.endswith(unit):
                report["first_frame_ms"].append(float(line[:-len(unit)]) * multiplier)
                break
        else:
            raise RuntimeError(f"unrecognized startup duration: {line}")
    report["median_first_frame_ms"] = statistics.median(report["first_frame_ms"])
    checked([str(binary), "--help"], capture_output=True)
    fixture = output / "rendering.md"
    icon = Path("assets/icon.png").resolve().as_posix()
    fixture.write_text("# Native rendering validation\n\nUnicode: 日本語 — **bold** and `code`.\n\n"
                       "| Column | Value |\n| --- | --- |\n| Text | ✓ |\n\n"
                       "$$\n\\frac{a+b}{c-d} = \\sqrt{x^2+y^2}\n$$\n\n"
                       "```mermaid\nflowchart LR\n A[Native GPU] --> B[Rendered Markdown]\n```\n\n"
                       f"![Application icon](<{icon}>)\n", encoding="utf-8")
    # Fresh processes must pick up each OS theme; allow background math/diagrams
    # to complete before capturing the actual desktop for visual inspection.
    prior_theme = None
    if sys.platform == "darwin":
        prior = checked(["osascript", "-e", 'tell application "System Events" to get dark mode of appearance preferences'], capture_output=True, text=True)
        prior_theme = prior.stdout.strip()
    elif sys.platform == "win32":
        import winreg
        with winreg.CreateKey(winreg.HKEY_CURRENT_USER, r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize") as key:
            try:
                prior_theme = winreg.QueryValueEx(key, "AppsUseLightTheme")
            except FileNotFoundError:
                pass
    try:
        for dark in [False, True]:
            theme = "dark" if dark else "light"
            if sys.platform == "darwin":
                checked(["osascript", "-e", 'tell application "System Events" to set dark mode of appearance preferences to ' + str(dark).lower()])
            elif sys.platform == "win32":
                import winreg
                with winreg.CreateKey(winreg.HKEY_CURRENT_USER, r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize") as key:
                    winreg.SetValueEx(key, "AppsUseLightTheme", 0, winreg.REG_DWORD, int(not dark))
                notify_windows_theme_change()
            else:
                raise RuntimeError("this script requires a native Windows or macOS desktop")
            with (output / f"{theme}-gui.log").open("w") as log:
                process = subprocess.Popen([str(binary), str(fixture)], stdout=log, stderr=log)
                try:
                    time.sleep(3)
                    if process.poll() is not None:
                        raise RuntimeError(f"{theme} native app exited before screenshot")
                    started = time.perf_counter()
                    checked([str(binary)], capture_output=True)
                    report[f"{theme}_warm_handoff_ms"] = (time.perf_counter() - started) * 1000
                    if sys.platform == "darwin":
                        checked(["screencapture", "-x", str(output / f"{theme}.png")])
                    else:
                        checked(["powershell", "-NoProfile", "-File", str(Path(__file__).with_name("capture.ps1")),
                                 "-AppProcessId", str(process.pid), "-OutputPath", str(output / f"{theme}.png"), "-ExpectedTitle", fixture.name])
                finally:
                    process.terminate()
                    process.wait(timeout=15)
    finally:
        if sys.platform == "darwin":
            checked(["osascript", "-e", 'tell application "System Events" to set dark mode of appearance preferences to ' + prior_theme])
        elif sys.platform == "win32":
            with winreg.CreateKey(winreg.HKEY_CURRENT_USER, r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize") as key:
                if prior_theme is None:
                    winreg.DeleteValue(key, "AppsUseLightTheme")
                else:
                    winreg.SetValueEx(key, "AppsUseLightTheme", 0, prior_theme[1], prior_theme[0])
            notify_windows_theme_change()
        (output / "report.json").write_text(json.dumps(report, indent=2))
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
