# Load .env
ifneq (,$(wildcard ./.env))
    include .env
    export
endif

.PHONY: lint
lint: # Run linter
	@echo "Run formatter check"
	@cargo fmt --check
	@uvx --from panache-cli==2.61.0 panache format --check .
	@echo "Run clippy"
	@cargo clippy --all-targets -- -D warnings
	@echo "Run checkrs"
	@uvx --from git+https://github.com/pyk/checkrs checkrs run src/

.PHONY: fmt
fmt: # Run formatter
	@echo "Run rust formatter"
	@cargo fmt
	@echo "Run markdown formatter"
	@uvx --from panache-cli==2.61.0 panache format .

FIXTURE_DIRS := $(wildcard fixtures/* fixtures/*/*)

.PHONY: build-fixtures
build-fixtures: # Force-rebuild all test fixtures with --ast
	@echo "Building fixtures"
	@for d in $(FIXTURE_DIRS); do \
		echo "  $$d"; \
		forge build --root "$$d" --ast --extra-output storageLayout --force --quiet || true; \
	done

.PHONY: bin
bin: # Install local binary
	@echo "Installing local binary"
	@cargo install --path . --locked

.PHONY: test
test: # Run tests
	@echo "Running tests"
	@cargo test --quiet -- --skip live

.PHONY: test-live
test-live: # Run tests against live network
	@echo "Running tests"
	@cargo test live

.PHONY: challenges
challenges: # Run challenge tests
	@echo "Running challenges"
	@cargo test --quiet --test max_challenges -- --ignored

# Catch-all target to handle extra arguments passed to make
%:
	@
