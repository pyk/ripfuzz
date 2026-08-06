# Load .env
ifneq (,$(wildcard ./.env))
    include .env
    export
endif

.PHONY: check
check: # Run code quality tools
	@echo "Run clippy"
	@cargo clippy -- -D warnings
	@echo "Run checkrs"
	@uvx --from git+https://github.com/pyk/checkrs checkrs run src/
	@echo "Run markdown formatter"
	@uvx --from panache-cli==2.61.0 panache format --check .

.PHONY: fmt
fmt: # Run code formatters
	@echo "Run rust formatter"
	@cargo fmt
	@echo "Run markdown formatter"
	@uvx --from panache-cli==2.61.0 panache format .

FIXTURE_DIRS := $(wildcard fixtures/*)

.PHONY: build-fixtures
build-fixtures: ## Force-rebuild all test fixtures with --ast
	@echo "Building fixtures"
	@for d in $(FIXTURE_DIRS); do \
		echo "  $$d"; \
		forge build --root "$$d" --ast --extra-output storageLayout --force --quiet || true; \
	done

.PHONY: bin
bin: ## Install local binary
	@echo "Installing local binary"
	@cargo install --path . --locked

.PHONY: test
test: ## Run tests
	@echo "Running tests"
	@cargo test --quiet -- --skip live

.PHONY: test-live
test-live: ## Run tests against live network
	@echo "Running tests"
	@cargo test live

# Catch-all target to handle extra arguments passed to make
%:
	@
