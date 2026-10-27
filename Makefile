BUILD_LOCAL := python3 $(CURDIR)/scripts/build-local.py
.PHONY: build check
build:
	$(BUILD_LOCAL) run --prepare-node --cargo-output-dir "$${CARGO_TARGET_DIR:-/Volumes/sam/build/rust/targets}" --output dist="$(CURDIR)/dist" -- sh -c 'pnpm build && cargo build -j4 --workspace'
check:
	$(BUILD_LOCAL) run --prepare-node -- sh -c 'pnpm build && cargo fmt --all -- --check && cargo test -j4 --workspace && cargo clippy -j4 --workspace --all-targets -- -D warnings'
