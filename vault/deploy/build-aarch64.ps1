# 交叉编译 aarch64 musl 静态二进制（香橙派 systemd 部署用）
# 前置：rustup target add aarch64-unknown-linux-musl
#       并安装 aarch64-linux-musl-gcc 交叉链接器（或用 cargo-zigbuild）

$env:RUSTFLAGS = "-C target-feature=+crt-static"
cargo build --release --target aarch64-unknown-linux-musl
Write-Host "产物: target\aarch64-unknown-linux-musl\release\agent-manager-vault"
