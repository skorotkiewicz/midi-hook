# https://github.com/casey/just

[private]
default:
    @just --list

build:
    cargo build --release

run *args:
    cargo run --release -- {{ args }}

fmt:
    cargo fmt
    cargo clippy --all-targets -- -D warnings

check:
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings

test: fmt
    cargo test
