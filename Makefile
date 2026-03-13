.PHONY: all build clean test debug

all: build

build:
	cargo build --workspace

clean:
	cargo clean

test:
	cargo test --workspace

debug:
	-pkill -f slated
	RUST_LOG=slated=debug cargo run -p slated
	cargo run -p slate -- --debug

release:
	cargo build --workspace --release
