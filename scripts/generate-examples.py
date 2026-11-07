"""Create deterministic sample PNGs and the two data-only example projects."""
from pathlib import Path
import copy
import json
import math
import re
import struct
import zlib

ROOT = Path(__file__).resolve().parents[1]


def png(path, width, height, pixel):
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    rows = bytearray()
    for y in range(height):
        rows.append(0)
        for x in range(width):
            rows.extend(pixel(x, y))
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(rows, 9)) + chunk(b'IEND', b''))


def character_pixel(x, y, frame=0, mascot=False):
    # Pixel-art fixture authored here; no external artwork or image libraries.
    x, y = x // 8, y // 8
    y += [0, 0, -1, 0, -1, 0, 0, -2, -3, -1, 0, -1, 0, -1, -1, 0][frame]
    fill = (255, 182, 100, 255) if mascot else (137, 172, 211, 255)
    outline = (42, 48, 68, 255)
    body = ((x - 16) / 10) ** 2 + ((y - 19) / 10) ** 2 <= 1
    ears = not mascot and ((6 <= x <= 12 and 5 <= y <= 13 and x - 6 <= y - 5) or (20 <= x <= 26 and 5 <= y <= 13 and 26 - x <= y - 5))
    if not (body or ears):
        return (0, 0, 0, 0)
    edge = ((x - 16) / 9) ** 2 + ((y - 19) / 9) ** 2 > 1 and not ears
    if 11 <= x <= 12 and 15 <= y <= 17 or 20 <= x <= 21 and 15 <= y <= 17:
        return outline
    if y == 21 and 15 <= x <= 17 or x == 16 and 19 <= y <= 21:
        return outline
    if frame in (2, 3, 14, 15) and y >= 25 and ((frame % 2 == 0 and x <= 12) or (frame % 2 and x >= 20)):
        return (246, 229, 199, 255)
    if frame in (4, 5, 6, 7, 8, 9) and y == 13 and 14 <= x <= 18:
        return (255, 239, 142, 255)
    return outline if edge else fill


def main():
    source = (ROOT / 'docs/project-format.md').read_text()
    project, manifest = [json.loads(value) for value in re.findall(r'```json\s*\n(.*?)\n```', source, re.S)]
    for slug, mascot in [('typing-cat', False), ('click-mascot', True)]:
        data, character = copy.deepcopy(project), copy.deepcopy(manifest)
        if mascot:
            data['projectId'] = '27e75963-e365-4fb7-97b5-f3029c9dd7a1'
            data['product'].update(slug=slug, name='Click Mascot', identifier='com.example.buddy.clickmascot')
            data['bindings'] = [binding for binding in data['bindings'] if binding['id'] == 'click']
            character.update(id='sample-mascot', name='Sample Mascot')
        folder = ROOT / 'examples' / slug
        (folder / 'character').mkdir(parents=True, exist_ok=True)
        (folder / 'buddy.project.json').write_text(json.dumps(data, ensure_ascii=False, indent=2) + '\n')
        (folder / 'character/manifest.json').write_text(json.dumps(character, ensure_ascii=False, indent=2) + '\n')
        (folder / 'character/LICENSE.txt').write_text('Original Desktop Buddy Studio example artwork.\nSPDX-License-Identifier: MIT\nCopyright (c) 2026 Desktop Buddy Studio contributors.\nPermission is granted to use, copy, modify, and distribute this artwork without restriction.\nThe artwork is provided AS IS, without warranty.\n')
        png(folder / 'character/sprite.png', 1024, 1024, lambda x, y: character_pixel(x % 256, y % 256, x // 256 + (y // 256) * 4, mascot))
        png(folder / 'assets/app-icon.png', 512, 512, lambda x, y: character_pixel(x // 2, y // 2, 0, mascot))
        print(f'Generated examples/{slug}')


if __name__ == '__main__':
    main()
