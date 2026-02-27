.PHONY: all build clean test

all: build

build:
	cargo build --workspace

clean:
	cargo clean

test:
	cargo test --workspace

release:
	cargo build --workspace --release
