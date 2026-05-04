.PHONY: all build clean test debug install uninstall

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

install: build
	@mkdir -p $(HOME)/.local/bin
	@ln -sf $(CURDIR)/target/debug/codiv $(HOME)/.local/bin/codiv
	@ln -sf $(CURDIR)/target/debug/codivd $(HOME)/.local/bin/codivd
	@echo "Installed symlinks to $(HOME)/.local/bin/{codiv,codivd}"
	@echo "Make sure $(HOME)/.local/bin is in your PATH"

uninstall:
	@rm -f $(HOME)/.local/bin/codiv $(HOME)/.local/bin/codivd
	@echo "Removed symlinks from $(HOME)/.local/bin"
