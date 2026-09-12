.DEFAULT_GOAL := help

.PHONY: help build test lint audit check run docker-build compose-up clean

help: ## List development commands
	@awk 'BEGIN {FS = ":.*## "} /^[a-zA-Z_-]+:.*## / {printf "%-16s %s\n", $$1, $$2}' $(MAKEFILE_LIST)

build: ## Build a debug binary
	cargo build --locked

test: ## Run all tests
	cargo test --locked --all-targets

lint: ## Enforce formatting and Clippy warnings
	cargo fmt --all -- --check
	cargo clippy --locked --all-targets --all-features -- -D warnings

audit: ## Scan the dependency graph with cargo-audit
	cargo audit

check: lint test ## Run the local merge gates

run: ## Start Waybend with the example configuration
	cargo run -- serve --config waybend.example.yml

docker-build: ## Build the production container
	docker build --tag waybend:dev .

compose-up: ## Start the local Compose deployment
	docker compose up --build

clean: ## Remove Rust build output
	cargo clean
