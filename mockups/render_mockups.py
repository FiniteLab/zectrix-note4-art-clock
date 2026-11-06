"""Render 400x300 NOTE4 mockups for the art clock concept.

device/  : continuous-tone grayscale PNGs, ready to push with the SDK `show` example (gray16).
preview/ : simulated 16-level e-ink look (Floyd-Steinberg, paper/ink tint), 2x nearest-neighbour.
"""
from PIL import Image, ImageDraw, ImageFont, ImageOps, ImageStat
import os

BASE = "/Users/soulspell/note4-art-clock/mockups"
A = f"{BASE}/assets"
OUT_DEV = f"{BASE}/device"
OUT_PRE = f"{BASE}/preview"
os.makedirs(OUT_DEV, exist_ok=True)
os.makedirs(OUT_PRE, exist_ok=True)
W, H = 400, 300
LZ = Image.LANCZOS

KAITI = "/System/Library/AssetsV2/com_apple_MobileAsset_Font8/88d6cc32a907955efa1d014207889413890573be.asset/AssetData/Kaiti.ttc"
SUPP = "/System/Library/Fonts/Supplemental"
FONTS = {
    "didot": (f"{SUPP}/Didot.ttc", 0),
    "didot_i": (f"{SUPP}/Didot.ttc", 1),
    "song": (f"{SUPP}/Songti.ttc", 6),
    "song_b": (f"{SUPP}/Songti.ttc", 1),
    "song_l": (f"{SUPP}/Songti.ttc", 3),
    "kai": (KAITI, 0),
    "kai_b": (KAITI, 3),
}


def font(name, size):
    path, idx = FONTS[name]
    return ImageFont.truetype(path, size, index=idx)


def art(name, cutoff=1):
    im = Image.open(f"{A}/{name}.jpg").convert("L")
    return ImageOps.autocontrast(im, cutoff=cutoff)


NO_LINE_START = set("，。、；：！？）》」』”’·…")


def wrap(text, fnt, max_w):
    lines, cur = [], ""
    for ch in text:
        trial = cur + ch
        if not cur or fnt.getlength(trial) <= max_w:
            cur = trial
            continue
        if ch in NO_LINE_START:
            lines.append(cur[:-1])
            cur = cur[-1] + ch
        else:
            lines.append(cur)
            cur = ch
    if cur:
        lines.append(cur)
    return lines


def vtext(d, cx, top, s, fnt, fill=0, step=None):
    step = step or int(fnt.size * 1.08)
    y = top
    for ch in s:
        d.text((cx, y), ch, font=fnt, fill=fill, anchor="mt")
        y += step
    return y


def hairline_frame(d, x, y, w, h, gap=4, fill=0):
    d.rectangle([x - gap - 1, y - gap - 1, x + w + gap, y + h + gap], outline=fill, width=1)


def play_icon(d, x, y, size=8, fill=0):
    d.polygon([(x, y - size // 2), (x, y + size // 2), (x + int(size * 0.87), y)], fill=fill)


# ---------- screens ----------

def s1_framed():
    c = Image.new("L", (W, H), 255)
    d = ImageDraw.Draw(c)
    a = ImageOps.contain(art("wave_large"), (300, 196), LZ)
    x, y = (W - a.width) // 2, 16
    c.paste(a, (x, y))
    hairline_frame(d, x, y, a.width, a.height)
    d.text((26, 284), "14:35", font=font("didot", 56), fill=0, anchor="ls")
    d.line([(186, 240), (186, 286)], fill=0, width=1)
    d.text((200, 251), "十一月四日 · 星期三", font=font("song", 13), fill=0, anchor="ls")
    d.text((200, 270), "葛饰北斋《神奈川冲浪里》", font=font("song_b", 13), fill=0, anchor="ls")
    d.text((200, 286), "1830–33 · 木版画", font=font("song", 11), fill=70, anchor="ls")
    return c


def quietest_corner(img, tw, th, m=10):
    corners = {
        "tl": (m, m), "tr": (W - tw - m, m),
        "bl": (m, H - th - m), "br": (W - tw - m, H - th - m),
    }
    scores = {k: ImageStat.Stat(img.crop((x, y, x + tw, y + th))).stddev[0] for k, (x, y) in corners.items()}
    k = min(scores, key=scores.get)
    return corners[k]


def s2_fullbleed():
    c = ImageOps.fit(art("rainy"), (W, H), LZ, centering=(0.5, 0.45))
    d = ImageDraw.Draw(c)
    tw, th = 132, 66
    x, y = quietest_corner(c, tw, th)
    d.rounded_rectangle([x, y, x + tw, y + th], radius=8, fill=255, outline=90, width=1)
    d.text((x + 12, y + 40), "10:24", font=font("didot", 36), fill=0, anchor="ls")
    d.text((x + 12, y + 58), "周三 · 小雨 12°", font=font("song", 12), fill=40, anchor="ls")
    cap = "雨天配画 · 卡耶博特《巴黎街道，雨天》"
    f = font("song", 10)
    cw = int(f.getlength(cap)) + 14
    cx, cy = W - cw - 8, H - 24
    d.rounded_rectangle([cx, cy, cx + cw, cy + 17], radius=4, fill=255)
    d.text((cx + 7, cy + 13), cap, font=f, fill=30, anchor="ls")
    return c


def s3_shichen():
    c = Image.new("L", (W, H), 255)
    d = ImageDraw.Draw(c)
    a = ImageOps.contain(art("mishima"), (300, 220), LZ)
    x, y = 14, 36
    c.paste(a, (x, y))
    hairline_frame(d, x, y, a.width, a.height, gap=3)
    d.text((x, y + a.height + 26), "歌川广重《东海道五十三次·三岛 朝雾》", font=font("song", 12), fill=40, anchor="ls")
    d.line([(328, 22), (328, 278)], fill=0, width=1)
    vtext(d, 372, 22, "辰初", font("kai_b", 44), step=48)
    vtext(d, 372, 124, "三刻", font("kai", 30), step=33)
    d.text((372, 210), "07:50", font=font("didot", 17), fill=0, anchor="ms")
    vtext(d, 344, 26, "立冬前三日", font("kai", 13), fill=60, step=15)
    sx, sy, ss = 358, 234, 28
    d.rectangle([sx, sy, sx + ss, sy + ss], fill=0)
    sf = font("kai_b", 12)
    d.text((sx + ss // 2, sy + 3), "画", font=sf, fill=255, anchor="mt")
    d.text((sx + ss // 2, sy + 15), "时", font=sf, fill=255, anchor="mt")
    return c


def s4_guide():
    c = Image.new("L", (W, H), 255)
    d = ImageDraw.Draw(c)
    d.text((16, 22), "导览", font=font("song_b", 12), fill=0, anchor="ls")
    d.text((384, 22), "1 / 3", font=font("didot", 13), fill=0, anchor="rs")
    d.line([(16, 30), (384, 30)], fill=0, width=1)
    a = ImageOps.contain(art("wave_large"), (150, 120), LZ)
    c.paste(a, (18, 46))
    hairline_frame(d, 18, 46, a.width, a.height, gap=3)
    d.text((18, 46 + a.height + 22), "芝加哥艺术博物馆藏", font=font("song", 10), fill=70, anchor="ls")
    d.text((18, 46 + a.height + 36), "公有领域 · CC0", font=font("song", 10), fill=70, anchor="ls")
    rx, rw = 186, 198
    d.text((rx, 62), "神奈川冲浪里", font=font("song_b", 22), fill=0, anchor="ls")
    d.text((rx, 82), "葛饰北斋 · 1830–33 · 木版画", font=font("song", 11), fill=60, anchor="ls")
    body = "《富岳三十六景》里最有名的一幅。巨浪像张开的爪子扑向三艘小船，远处的富士山反而小得像一朵浪花。北斋大胆用上了刚从欧洲传入的普鲁士蓝。"
    bf = font("song", 14)
    yy = 110
    for line in wrap(body, bf, rw):
        d.text((rx, yy), line, font=bf, fill=0, anchor="ls")
        yy += 22
    d.line([(16, 262), (384, 262)], fill=0, width=1)
    play_icon(d, 18, 280, 9)
    d.text((32, 284), "正在讲解", font=font("song_b", 12), fill=0, anchor="ls")
    d.text((384, 284), "下键 看细节　确认 停止", font=font("song", 11), fill=60, anchor="rs")
    return c


def s5_detail():
    c = Image.new("L", (W, H), 255)
    d = ImageDraw.Draw(c)
    big = art("wave_large")
    bw, bh = big.size
    crop = big.crop((int(bw * 0.41), int(bh * 0.51), int(bw * 0.81), int(bh * 0.84)))
    crop = ImageOps.fit(crop, (W, 228), LZ)
    c.paste(crop, (0, 0))
    d.line([(0, 228), (W, 228)], fill=0, width=1)
    d.text((16, 256), "浪谷里的富士山", font=font("song_b", 18), fill=0, anchor="ls")
    d.text((384, 256), "2 / 3", font=font("didot", 13), fill=60, anchor="rs")
    d.text((384 - 34, 256), "细节", font=font("song", 11), fill=60, anchor="rs")
    d.text((16, 282), "雪顶的富士山被浪谷框住，整幅画的动势都指向它。", font=font("song", 13), fill=20, anchor="ls")
    return c


def s6_night():
    c = Image.new("L", (W, H), 0)
    d = ImageDraw.Draw(c)
    a = ImageOps.contain(art("moonrise", cutoff=0.5), (196, 270), LZ)
    x, y = W - a.width - 18, (H - a.height) // 2
    c.paste(a, (x, y))
    hairline_frame(d, x, y, a.width, a.height, gap=4, fill=150)
    d.text((22, 118), "23:45", font=font("didot", 54), fill=255, anchor="ls")
    d.text((24, 144), "夜间 · 每 15 分钟更新", font=font("song", 11), fill=170, anchor="ls")
    d.text((24, 214), "晚安", font=font("kai", 30), fill=235, anchor="ls")
    d.text((24, 278), "英尼斯《月出》1891", font=font("song", 10), fill=150, anchor="ls")
    return c


def s7_framed_big():
    c = Image.new("L", (W, H), 255)
    d = ImageDraw.Draw(c)
    a = ImageOps.contain(art("wave_large"), (250, 146), LZ)
    x, y = (W - a.width) // 2, 14
    c.paste(a, (x, y))
    hairline_frame(d, x, y, a.width, a.height)
    d.text((W // 2, 266), "14:35", font=font("didot", 108), fill=0, anchor="ms")
    d.text((W // 2, 290), "十一月四日 星期三 · 葛饰北斋《神奈川冲浪里》", font=font("song", 12), fill=50, anchor="ms")
    return c


def s8_split_stacked():
    c = Image.new("L", (W, H), 255)
    d = ImageDraw.Draw(c)
    a = ImageOps.fit(art("wave_large"), (214, 300), LZ, centering=(0.32, 0.5))
    c.paste(a, (0, 0))
    d.line([(214, 0), (214, H)], fill=0, width=1)
    cx = 214 + (W - 214) // 2
    tf = font("didot", 118)
    d.text((cx, 120), "14", font=tf, fill=0, anchor="ms")
    d.text((cx, 224), "35", font=tf, fill=0, anchor="ms")
    d.line([(cx - 40, 248), (cx + 40, 248)], fill=0, width=1)
    d.text((cx, 270), "十一月四日 · 周三", font=font("song", 13), fill=0, anchor="ms")
    d.text((cx, 288), "葛饰北斋《神奈川冲浪里》", font=font("song", 11), fill=60, anchor="ms")
    return c


def s9_fullbleed_band():
    c = Image.new("L", (W, H), 255)
    d = ImageDraw.Draw(c)
    band = 96
    a = ImageOps.fit(art("rainy"), (W, H - band), LZ, centering=(0.5, 0.4))
    c.paste(a, (0, 0))
    d.line([(0, H - band), (W, H - band)], fill=0, width=1)
    d.text((16, 286), "10:24", font=font("didot", 92), fill=0, anchor="ls")
    rx = 272
    d.line([(rx - 12, H - band + 18), (rx - 12, H - 14)], fill=0, width=1)
    d.text((rx, 236), "周三", font=font("song_b", 16), fill=0, anchor="ls")
    d.text((rx, 258), "小雨 12°", font=font("song", 14), fill=0, anchor="ls")
    d.text((rx, 280), "雨天配画 · 卡耶博特", font=font("song", 10), fill=60, anchor="ls")
    return c


def s10_shichen_big():
    c = Image.new("L", (W, H), 255)
    d = ImageDraw.Draw(c)
    a = ImageOps.contain(art("mishima"), (252, 200), LZ)
    x, y = 14, (H - a.height) // 2 - 8
    c.paste(a, (x, y))
    hairline_frame(d, x, y, a.width, a.height, gap=3)
    d.text((x, y + a.height + 24), "歌川广重《三岛 朝雾》", font=font("song", 12), fill=40, anchor="ls")
    d.line([(282, 18), (282, 282)], fill=0, width=1)
    cx = 342
    vtext(d, cx, 14, "辰初", font("kai_b", 64), step=66)
    vtext(d, cx, 150, "三刻", font("kai", 44), step=46)
    d.text((cx, 270), "07:50", font=font("didot", 26), fill=0, anchor="ms")
    vtext(d, 300, 22, "立冬前三日", font("kai", 12), fill=70, step=14)
    return c


# ---------- e-ink preview ----------

def eink_preview(img, scale=2):
    pal = Image.new("P", (1, 1))
    levels = [round(i * 255 / 15) for i in range(16)]
    flat = []
    for v in levels:
        flat += [v, v, v]
    flat += [0] * (768 - len(flat))
    pal.putpalette(flat)
    q = img.convert("RGB").quantize(palette=pal, dither=Image.Dither.FLOYDSTEINBERG).convert("L")
    tinted = ImageOps.colorize(q, black=(34, 34, 36), white=(236, 233, 224))
    return tinted.resize((W * scale, H * scale), Image.NEAREST)


SCREENS = [
    ("01_framed", "画框模式（默认）", s1_framed),
    ("02_fullbleed", "满幅模式 + 画随天时", s2_fullbleed),
    ("03_shichen", "时辰模式（东方作品）", s3_shichen),
    ("04_guide", "导览页（短按确认）", s4_guide),
    ("05_detail", "细节镜头（导览中按下键）", s5_detail),
    ("06_night", "夜间模式", s6_night),
    ("07_framed_big", "大钟 A：画框 + 居中大钟", s7_framed_big),
    ("08_split_stacked", "大钟 B：左画右钟，时分上下叠放", s8_split_stacked),
    ("09_fullbleed_band", "大钟 C：画在上，底栏大钟", s9_fullbleed_band),
    ("10_shichen_big", "大钟 D：时辰大字", s10_shichen_big),
]

def render_all():
    previews = {}
    for key, label, fn in SCREENS:
        img = fn()
        assert img.size == (W, H)
        img.save(f"{OUT_DEV}/{key}.png")
        eink_preview(img).save(f"{OUT_PRE}/{key}.png")
        previews[key] = (label, eink_preview(img, scale=1))
    keys = [s[0] for s in SCREENS]
    contact_sheet(previews, keys[:6], f"{BASE}/overview.png")
    contact_sheet(previews, keys[6:], f"{BASE}/overview_big_clock.png", start=7)
    print("ok", keys)


def contact_sheet(previews, keys, path, start=1):
    gap, cap_h, cols = 24, 30, 2
    rows = (len(keys) + cols - 1) // cols
    sheet = Image.new("RGB", (cols * W + (cols + 1) * gap, rows * (H + cap_h) + (rows + 1) * gap - 6), (250, 249, 246))
    sd = ImageDraw.Draw(sheet)
    cf = font("song_b", 15)
    for i, k in enumerate(keys):
        label, p = previews[k]
        r, col = divmod(i, cols)
        x = gap + col * (W + gap)
        y = gap + r * (H + cap_h + gap)
        sd.text((x, y + 18), f"{start + i}. {label}", font=cf, fill=(30, 30, 30), anchor="ls")
        sheet.paste(p, (x, y + cap_h))
        sd.rectangle([x - 1, y + cap_h - 1, x + W, y + cap_h + H], outline=(170, 170, 170))
    sheet.save(path)


if __name__ == "__main__":
    render_all()
