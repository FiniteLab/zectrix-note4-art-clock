"""Compare redistributable (OFL) numeral fonts against Apple Didot in layout C."""
from PIL import Image, ImageDraw, ImageFont, ImageOps
from render_mockups import W, H, LZ, BASE, OUT_DEV, OUT_PRE, FONTS, font, art, eink_preview, contact_sheet

F = "/Users/soulspell/note4-art-clock/app/assets/fonts"
C = "/Users/soulspell/note4-art-clock/mockups/assets/font-candidates"
FONTS.update({
    "shs": (f"{F}/SourceHanSansCN-Regular.otf", 0),
    "shs_b": (f"{F}/SourceHanSansCN-Bold.otf", 0),
    "pixel": ("/Users/soulspell/Library/Fonts/fusion-pixel-12px-proportional.otf", 0),
})


def numeral_font(kind, size):
    if kind == "didot":
        return font("didot", size)
    if kind == "gfs":
        return ImageFont.truetype(f"{F}/GFSDidot-Regular.ttf", size)
    if kind == "playfair":
        return ImageFont.truetype(f"{C}/PlayfairDisplay.ttf", size)
    if kind == "bodoni":
        f = ImageFont.truetype(f"{C}/BodoniModa.ttf", size)
        axes = f.get_variation_axes()
        vals = []
        for a in axes:
            name = a["name"] if isinstance(a["name"], str) else a["name"].decode()
            vals.append(96 if "ptical" in name else 400)
        f.set_variation_by_axes(vals)
        return f
    raise ValueError(kind)


def layout_c(kind, art_name="rainy", centering=(0.5, 0.4), time_s="10:24",
             info=("周三", "小雨 12°", "卡耶博特"), foot="1877 · 芝加哥艺术博物馆"):
    c = Image.new("L", (W, H), 255)
    d = ImageDraw.Draw(c)
    band = 96
    a = ImageOps.fit(art(art_name), (W, H - band), LZ, centering=centering)
    c.paste(a, (0, 0))
    d.line([(0, H - band), (W, H - band)], fill=0, width=1)
    d.text((14, 284), time_s, font=numeral_font(kind, 92), fill=0, anchor="ls")
    rx = 270
    d.line([(rx - 12, H - band + 14), (rx - 12, H - 12)], fill=0, width=1)
    d.text((rx, 228), info[0], font=font("shs_b", 18), fill=0, anchor="ls")
    d.text((rx, 251), info[1], font=font("shs", 16), fill=0, anchor="ls")
    d.text((rx, 272), info[2], font=font("shs", 14), fill=0, anchor="ls")
    d.fontmode = "1"
    d.text((rx, 290), foot, font=font("pixel", 12), fill=0, anchor="ls")
    d.fontmode = "L"
    return c


SCREENS = [
    ("30_num_didot", "Didot（macOS 自带，不能分发）", lambda: layout_c("didot")),
    ("31_num_gfs", "GFS Didot（OFL）", lambda: layout_c("gfs")),
    ("32_num_playfair", "Playfair Display（OFL）", lambda: layout_c("playfair")),
    ("33_num_bodoni", "Bodoni Moda（OFL）", lambda: layout_c("bodoni")),
]

if __name__ == "__main__":
    previews = {}
    for key, label, fn in SCREENS:
        img = fn()
        img.save(f"{OUT_DEV}/{key}.png")
        eink_preview(img).save(f"{OUT_PRE}/{key}.png")
        previews[key] = (label, eink_preview(img, scale=1))
    contact_sheet(previews, [s[0] for s in SCREENS], f"{BASE}/overview_numerals.png")
    print("ok")
