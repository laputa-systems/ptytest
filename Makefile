.PHONY: lint bump

lint:
	cargo fmt --all
	cargo clippy --fix --allow-dirty --all-targets --all-features -- --deny warnings

bump:
	./tools/bump_minor_release.sh
