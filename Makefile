.DEFAULT_GOAL := help

.PHONY: help build run serve fmt fmt-check clippy test rust-test fast-test swift-build swift-test check

help:
	@printf '%s\n' \
		'make build        Build the Rust workspace and Swift helper' \
		'make run          Run the nd terminal client' \
		'make serve        Run the nd Control API Server' \
		'make fmt          Format the Rust workspace' \
		'make fmt-check    Check Rust formatting without changing files' \
		'make clippy       Lint all Rust targets and features' \
		'make test         Run Rust and Swift tests' \
		'make rust-test    Run all Rust tests and features' \
		'make fast-test    Run Rust tests with nextest (hung tests are killed)' \
		'make swift-build  Build the Swift helper' \
		'make swift-test   Test the Swift helper' \
		'make check        Run the complete Rust and Swift gate'

build:
	cargo build --workspace
	swift build --package-path helpers/apple-bridge

run:
	cargo run -p nd

serve:
	cargo run -p nd -- serve

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

clippy:
	cargo clippy --all-targets --all-features -- -D warnings

test: rust-test swift-test

rust-test:
	cargo test --all-features --quiet

fast-test:
	cargo nextest run --all-features
	cargo test --all-features --doc --quiet

swift-build:
	swift build --package-path helpers/apple-bridge

swift-test:
	swift test --package-path helpers/apple-bridge

check:
	cargo fmt --all -- --check
	cargo clippy --all-targets --all-features -- -D warnings
	cargo test --all-features --quiet
	swift build --package-path helpers/apple-bridge
	swift test --package-path helpers/apple-bridge
