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
	@echo "Running maxer challenges"
	@cargo test --quiet --test maxer -- --ignored challenges
	@echo "Running tester challenges"
	@cargo test --quiet --test tester -- --ignored challenges

.PHONY: doc
doc: # Build docs and serve them
	@echo "Run doc build"
	@cargo doc --no-deps
	@IP=$$(python3 -c 'import socket; s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.connect(("8.8.8.8", 80)); print(s.getsockname()[0])'); \
	echo "Serving docs on http://$$IP:8000/ripfuzz/"; \
	cd target/doc && python3 -m http.server 8000 --bind 0.0.0.0

# Catch-all target to handle extra arguments passed to make
%:
	@
