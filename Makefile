# Load .env
ifneq (,$(wildcard ./.env))
    include .env
    export
endif

.PHONY: check
check: # Run code quality tools
	@echo "Run formatter"
	@cargo fmt
	@echo "Run clippy"
	@cargo clippy -- -D warnings
	@echo "Run checkrs"
	@checkrs run src/
	@echo "Run flowmark"
	@uvx --from flowmark==0.7.2 flowmark -w 88 --list-spacing tight --nobackup -c --inplace .

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
