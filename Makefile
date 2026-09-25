.PHONY: build check
build:
	pnpm build
	cargo build -j4 --workspace
check:
	pnpm build
	cargo fmt --all -- --check
	cargo test -j4 --workspace
	cargo clippy -j4 --workspace --all-targets -- -D warnings
