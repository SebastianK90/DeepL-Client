#!/usr/bin/env python3
"""Check an isolated debug Quick Assist with the actual GTK/WebKit runtime.

Requires KDE, xdotool, ImageMagick, and Tesseract (deu+eng).
Pass --exe target/debug/deepl-client, or an extracted AppDir containing that
same debug binary, to check the libraries shipped in an AppImage. No real
clipboard, translation request, or user profile is used.
"""
import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import time


def check_image(path):
    text = subprocess.check_output(
        ['tesseract', str(path), 'stdout', '-l', 'deu+eng', '--psm', '6'], text=True
    )
    for label in ['Kopieren', 'Bearbeiten']:
        assert label in text, f'Quick Assist action is clipped or missing: {label}'
    width, height = map(int, subprocess.check_output(
        ['identify', '-format', '%w %h', str(path)], text=True).split())
    left, top = int(width * .72), 8
    crop_width, crop_height = width - 8 - left, int(height * .26) - top
    mean = float(subprocess.check_output([
        'magick', str(path), '-crop', f'{crop_width}x{crop_height}+{left}+{top}',
        '+repage', '-colorspace', 'Gray', '-threshold', '63%',
        '-format', '%[fx:mean]', 'info:'
    ], text=True))
    assert (1 - mean) * crop_width * crop_height > 40, \
        'Quick Assist title-bar controls are clipped or missing'
    return text


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--exe', type=Path, required=True)
    parser.add_argument('--name', default='native')
    args = parser.parse_args()
    assert args.name.replace('-', '').replace('_', '').isalnum()
    root = Path(__file__).resolve().parent.parent
    profile = root / '.tools' / 'quick-layout-native-check' / args.name
    profile.mkdir(parents=True, exist_ok=True)
    (profile / 'config.toml').write_text('provider="web"\nui_language="de"\ntheme="light"\nhistory_enabled=false\n')
    env = dict(os.environ, DEEPL_CLIENT_TEST_DIR=str(profile),
               DEEPL_CLIENT_UI_TEST='1', DEEPL_CLIENT_PLACEMENT_TEST='1',
               XDG_CONFIG_HOME=str(profile / 'config'), XDG_DATA_HOME=str(profile / 'data'),
               XDG_CACHE_HOME=str(profile / 'cache'))
    log = (profile / 'process.log').open('w')
    process = subprocess.Popen([str(args.exe.resolve())], env=env, stdout=log,
                               stderr=log, start_new_session=True)

    def bus(*arguments):
        return json.loads(subprocess.check_output(
            ['busctl', '--user', '--json=short', *arguments]))['data']

    def descendant(pid):
        for _ in range(15):
            if pid == process.pid:
                return True
            try:
                pid = int(Path(f'/proc/{pid}/status').read_text().split('PPid:')[1].splitlines()[0])
            except (FileNotFoundError, IndexError):
                return False
            if pid < 2:
                return False
        return False

    def xdo(*arguments):
        return subprocess.check_output(['xdotool', *map(str, arguments)], text=True).strip()

    try:
        time.sleep(4)
        items = bus('get-property', 'org.kde.StatusNotifierWatcher', '/StatusNotifierWatcher',
                    'org.kde.StatusNotifierWatcher', 'RegisteredStatusNotifierItems')
        for item in items:
            service, _, _ = item.partition('/')
            pid = bus('call', 'org.freedesktop.DBus', '/org/freedesktop/DBus',
                      'org.freedesktop.DBus', 'GetConnectionUnixProcessID', 's', service)[0]
            if not descendant(pid):
                continue
            windows = {xdo('getwindowname', xid): xid
                       for xid in xdo('search', '--pid', pid).splitlines()}
            quick = windows['Quick Assist']
            xdo('windowmap', quick)
            xdo('windowmove', quick, 100, 100)
            xdo('windowactivate', '--sync', quick)
            time.sleep(.3)
            initial = dict(line.split('=', 1) for line in
                           xdo('getwindowgeometry', '--shell', quick).splitlines())
            settings = next(w for w in json.loads((root / 'tauri.conf.json').read_text())
                            ['app']['windows'] if w['label'] == 'quick')
            assert int(initial['WIDTH']) >= settings['width']
            assert int(initial['HEIGHT']) >= settings['height']
            subprocess.run(['import', '-window', quick, str(profile / 'quick-default.png')], check=True)
            check_image(profile / 'quick-default.png')
            xdo('windowsize', quick, 300, 180)
            time.sleep(.7)
            geometry = dict(line.split('=', 1) for line in
                            xdo('getwindowgeometry', '--shell', quick).splitlines())
            minimum = next(w for w in json.loads((root / 'tauri.conf.json').read_text())
                           ['app']['windows'] if w['label'] == 'quick')
            # Derive native pixel scale from the fresh configured default.
            expected_minimum = (
                round(minimum['minWidth'] * int(initial['WIDTH']) / settings['width']),
                round(minimum['minHeight'] * int(initial['HEIGHT']) / settings['height']),
            )
            assert abs(int(geometry['WIDTH']) - expected_minimum[0]) <= 1
            assert abs(int(geometry['HEIGHT']) - expected_minimum[1]) <= 1
            screenshot = profile / 'quick.png'
            subprocess.run(['import', '-window', quick, str(screenshot)], check=True)
            text = check_image(screenshot)
            (profile / 'result.json').write_text(json.dumps(
                {'default_geometry': initial, 'geometry': geometry, 'visible_action_text': text, 'minimum_enforced': True, 'minimum_expected_physical': expected_minimum}, indent=2))
            print('Native Quick Assist passed: minimum size, title controls and action row.')
            break
        else:
            raise AssertionError('No tray item owned by the isolated test application')
    finally:
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGTERM)
        process.wait(timeout=5)
        log.close()


if __name__ == '__main__':
    main()
