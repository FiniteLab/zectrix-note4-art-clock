"""Small-text legibility test card and revised layout C for the NOTE4 (400x300, ~119 PPI)."""
from PIL import Image, ImageDraw, ImageOps
from render_mockups import W, H, LZ, BASE, OUT_DEV, OUT_PRE, FONTS, font, art, eink_preview, contact_sheet

FONTS.update({
    "hei": ("/System/Library/Fonts/Hiragino Sans GB.ttc", 0),
    "hei_b": ("/System/Library/Fonts/Hiragino Sans GB.ttc", 2),
    "pixel": ("/Users/soulspell/Library/Fonts/fusion-pixel-12px-proportional.otf", 0),
})

SAMPLE = "周三 小雨 12° 雨天配画 · 卡耶博特 1877"

# (row letter, font key, size, fill, antialias)
ROWS = [
    ("A", "song", 11, 60, True),
    ("B", "song", 11, 0, True),
    ("C", "song", 13, 0, True),
    ("D", "song_b", 13, 0, True),
    ("E", "song", 16, 0, True),
    ("F", "hei", 12, 0, True),
    ("G", "hei", 14, 0, True),
    ("H", "hei_b", 14, 0, True),
    ("I", "hei", 14, 0, False),
    ("J", "pixel", 12, 0, False),
]


def text_test():
    c = Image.new("L", (W, H), 255)
    d = ImageDraw.Draw(c)
    d.text((12, 19), "小字测试卡 · 哪几行清楚？", font=font("hei_b", 14), fill=0, anchor="ls")
    d.line([(12, 27), (388, 27)], fill=0, width=1)
    y = 30
    for letter, key, size, fill, aa in ROWS:
        base = y + 19
        d.fontmode = "L"
        d.text((12, base), letter, font=font("hei_b", 14), fill=0, anchor="ls")
        d.fontmode = "L" if aa else "1"
        d.text((34, base), SAMPLE, font=font(key, size), fill=fill, anchor="ls")
        y += 26
    d.fontmode = "L"
    return c


def layout_c(art_name, centering, time_s, info, info_fonts):
    c = Image.new("L", (W, H), 255)
    d = ImageDraw.Draw(c)
    band = 96
    a = ImageOps.fit(art(art_name), (W, H - band), LZ, centering=centering)
    c.paste(a, (0, 0))
    d.line([(0, H - band), (W, H - band)], fill=0, width=1)
    d.text((14, 286), time_s, font=font("didot", 92), fill=0, anchor="ls")
    rx = 270
    d.line([(rx - 12, H - band + 16), (rx - 12, H - 12)], fill=0, width=1)
    for (line, (fk, size)), base in zip(zip(info, info_fonts), (230, 256, 282)):
        d.text((rx, base), line, font=font(fk, size), fill=0, anchor="ls")
    return c


SCREENS = [
    ("20_text_test", "小字测试卡", text_test),
    ("21_c_hei", "C 改：黑体信息栏", lambda: layout_c(
        "rainy", (0.5, 0.4), "10:24", ["周三", "小雨 12°", "卡耶博特"],
        [("hei_b", 18), ("hei", 16), ("hei", 14)])),
    ("22_c_song", "C 改：粗宋信息栏", lambda: layout_c(
        "rainy", (0.5, 0.4), "10:24", ["周三", "小雨 12°", "卡耶博特"],
        [("song_b", 18), ("song_b", 16), ("song_b", 14)])),
    ("23_c_wave", "C 改：换成神奈川冲浪里", lambda: layout_c(
        "wave_large", (0.5, 0.45), "14:35", ["周三", "晴 16°", "葛饰北斋"],
        [("hei_b", 18), ("hei", 16), ("hei", 14)])),
]

if __name__ == "__main__":
    previews = {}
    for key, label, fn in SCREENS:
        img = fn()
        assert img.size == (W, H)
        img.save(f"{OUT_DEV}/{key}.png")
        eink_preview(img).save(f"{OUT_PRE}/{key}.png")
        previews[key] = (label, eink_preview(img, scale=1))
    contact_sheet(previews, [s[0] for s in SCREENS], f"{BASE}/overview_text_test.png", start=1)
    print("ok", [s[0] for s in SCREENS])
