.PHONY: check
check: ## Run code quality tools.
	@echo "Run formatter"
	@cargo fmt
	@echo "Run clippy"
	@cargo clippy -- -D warnings

.PHONY: bin
bin: ## Install local binary
	@echo "Installing local binary"
	@cargo install --path .

.PHONY: test
test: ## Run tests
	@echo "Running tests"
	@cargo test

# Catch-all target to handle extra arguments passed to make
%:
	@
