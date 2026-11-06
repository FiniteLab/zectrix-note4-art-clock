#!/bin/sh
# 在 NAS 上编译时钟，并装成 systemd 用户服务（开机自启，出错自动重启）。
# 适用于 Linux x86_64，不需要 sudo，也不需要系统自带的 C 编译器：
# C 编译器和链接器用 pip 装的 zig，CMake 也来自 pip，都放在 ~/note4-art-clock/.tools。
# 需要已装好 Rust（~/.cargo/bin）和 python3。
# 用法：sh ~/note4-art-clock/app/deploy/nas-build.sh <设备 IP>
set -eu
device=${1:?用法：sh nas-build.sh <设备 IP>}
base=$HOME/note4-art-clock
kit=$base/.tools/bin

if [ ! -x "$kit/cc-shim/cc" ]; then
    echo "准备编译工具（zig、cmake）…"
    python3 -m venv "$base/.tools"
    "$kit/pip" install -q --disable-pip-version-check ziglang cmake
    mkdir -p "$kit/cc-shim"
    cat > "$kit/cc-shim/cc" <<'SH'
#!/bin/sh
# zig cc rejects the Rust-style target triple that cc-rs passes.
for arg do
    shift
    case $arg in
        --target=x86_64-unknown-linux-gnu) arg=--target=x86_64-linux-gnu ;;
    esac
    set -- "$@" "$arg"
done
exec "$(dirname -- "$0")/../python" -m ziglang "${ZIG_TOOL:-cc}" "$@"
SH
    printf '#!/bin/sh\nZIG_TOOL=c++ exec "$(dirname -- "$0")/cc" "$@"\n' > "$kit/cc-shim/c++"
    printf '#!/bin/sh\nexec "$(dirname -- "$0")/../python" -m ziglang ar "$@"\n' > "$kit/cc-shim/ar"
    printf '#!/bin/sh\nexec "$(dirname -- "$0")/../python" -m ziglang ranlib "$@"\n' > "$kit/cc-shim/ranlib"
    chmod +x "$kit/cc-shim/cc" "$kit/cc-shim/c++" "$kit/cc-shim/ar" "$kit/cc-shim/ranlib"
fi

export PATH="$kit/cc-shim:$kit:$HOME/.cargo/bin:$PATH" CMAKE_POLICY_VERSION_MINIMUM=3.5
cd "$base/app"
echo "编译…"
cargo build --release

units=$HOME/.config/systemd/user
mkdir -p "$units"
printf 'ART_CLOCK_DEVICE=%s\n' "$device" > "$HOME/.config/art-clock.env"
cp deploy/art-clock.service "$units/"
# Without linger the user's services stop when the last SSH session ends.
loginctl enable-linger "$USER" 2>/dev/null ||
    echo "提示：没能开启 linger，退出登录后服务会停。请运行 sudo loginctl enable-linger $USER"
systemctl --user daemon-reload
systemctl --user enable --quiet art-clock.service
systemctl --user restart art-clock.service
echo "时钟服务已启动，设备 $device。日志：$base/art-clock.log"
echo "停止：systemctl --user stop art-clock    查看：systemctl --user status art-clock"
