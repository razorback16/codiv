.PHONY: all build clean test debug

all: build

build:
	cargo build --workspace

clean:
	cargo clean

test:
	cargo test --workspace

debug:
	@pkill codivd 2>/dev/null || true
	RUST_LOG=codivd=debug cargo run -p codivd
	cargo run -p codiv -- --debug

release:
	cargo build --workspace --release
