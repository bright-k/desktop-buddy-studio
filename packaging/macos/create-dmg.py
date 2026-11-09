"""Write the Finder layout directly so the build works without a GUI session."""

import json
from pathlib import Path
import sys
import unicodedata
from tempfile import TemporaryDirectory
from zipfile import ZipFile


def build(settings_path):
    config = json.loads(settings_path.read_text(encoding="utf-8"))
    app = Path(config["app"])
    files = [str(app)]
    locations = {app.name: config["appPosition"], "Applications": config["applicationsPosition"]}
    if config["guide"]:
        guide = Path(config["guide"])
        files.append(str(guide))
        locations[guide.name] = config["guidePosition"]
    icon = app / "Contents/Resources/icon.icns"
    background = config["background"]
    background_name = ".background" + (Path(background).suffix if background else ".tiff")
    hidden_names = [background_name, *([".VolumeIcon.icns"] if icon.is_file() else [])]
    # Finder can show dotfiles globally. Park them beyond the initial scroll
    # area so they never compete with the app, Applications or the guide.
    for index, name in enumerate(hidden_names):
        locations[name] = {"x": config["width"] + 192 + 160 * index, "y": config["height"] + 192}
    with TemporaryDirectory(prefix="buddy-dmg-layout-") as lib:
        for wheel in sorted(Path(__file__).with_name("wheels").glob("*.whl")):
            with ZipFile(wheel) as archive:
                archive.extractall(lib)
        sys.path.insert(0, lib)
        from dmgbuild import build_dmg

        build_dmg(config["output"], config["volume"], settings={
            "format": "UDZO",
            "files": files,
            "symlinks": {"Applications": "/Applications"},
            "icon": str(icon) if icon.is_file() else None,
            "background": background or "builtin-arrow",
            "hide": hidden_names,
            "window_rect": ((120, 100), (config["width"], config["height"])),
            # HFS+ exposes decomposed filenames. Finder looks up the actual
            # name in .DS_Store, including decomposed Hangul in app names.
            "icon_locations": {
                unicodedata.normalize("NFD", name): (point["x"], point["y"])
                for name, point in locations.items()
            },
            "icon_size": 108,
            "text_size": 13,
            "show_toolbar": False,
            "show_status_bar": False,
            "show_pathbar": False,
            "show_sidebar": False,
        })


if __name__ == "__main__":
    build(Path(sys.argv[1]))
