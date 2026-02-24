.PHONY: all slate slated clean test

all: slated slate

slate: slated
	cd slate && cargo build

slated:
	mkdir -p slated/build
	cd slated/build && cmake .. && cmake --build .

clean:
	cd slate && cargo clean
	rm -rf slated/build

test: all
	cd slate && cargo test
	cd slated/build && ctest --output-on-failure
