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
