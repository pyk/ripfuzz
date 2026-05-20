.PHONY: check
check: ## Run code quality tools.
	@echo "Run formatter"
	@cargo fmt
	@echo "Run clippy"
	@cargo clippy -- -D warnings
	@echo "Run checkrs"
	@checkrs run src/

FIXTURE_DIRS := $(wildcard fixtures/*)

.PHONY: build-fixtures
build-fixtures: ## Force-rebuild all test fixtures with --ast
	@echo "Building fixtures"
	@for d in $(FIXTURE_DIRS); do \
		echo "  $$d"; \
		forge build --root "$$d" --ast --force || true; \
	done

.PHONY: bin
bin: ## Install local binary
	@echo "Installing local binary"
	@cargo install --path . --locked

.PHONY: test
test: ## Run tests (120s suite timeout, single-threaded to avoid parallel LibAFL interference)
	@echo "Running tests"
	@cargo test --quiet

# Catch-all target to handle extra arguments passed to make
%:
	@
