#!/bin/sh
# 커밋 전 검사: 하나라도 실패하면 멈춘다 (CI 의 「시험」 과 같음 + 앱 clippy)
set -e
cd "$(dirname "$0")/.."
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test
echo "검사 통과"
