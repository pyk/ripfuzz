.PHONY: check
check: ## Run code quality tools.
	@echo "Run formatter"
	@cargo fmt
	@echo "Run clippy"
	@cargo clippy -- -D warnings
	@echo "Run checkrs"
	@checkrs run src/

.PHONY: bin
bin: ## Install local binary
	@echo "Installing local binary"
	@cargo install --path .

.PHONY: test
test: ## Run tests (120s suite timeout, single-threaded to avoid parallel LibAFL interference)
	@echo "Running tests"
	@timeout 120 cargo test -- --test-threads=1

.PHONY: test-nextest
test-nextest: ## Run tests with nextest (30s per-test timeout)
	@echo "Running tests with nextest"
	@cargo nextest run

# Catch-all target to handle extra arguments passed to make
%:
	@
