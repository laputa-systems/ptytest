.PHONY: test lint coverage publish

test:
	cargo test --locked

lint:
	cargo clippy --locked --all-targets --all-features -- --deny warnings

coverage:
	@mkdir -p .work/coverage
	cargo llvm-cov --locked --branch --json --summary-only \
		--ignore-filename-regex '(^|/)tests/' --output-path .work/coverage/summary.json

publish:
	./tools/bump_minor_release.sh

PYTHON ?= python3

.PHONY: mutants-diff
mutants-diff:
	$(PYTHON) -B tools/mutants_gate.py "$(or $(BASE),origin/main)"

.PHONY: clean
clean:
	@if test -L ./target || { test -e ./target && ! test -d ./target; }; then \
	  printf '%s\n' 'make clean: ./target must be a local build directory' >&2; exit 1; \
	fi
	rm -rf -- ./target
