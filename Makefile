.PHONY: build release test clean

# Auto-generate version from today's date, incrementing patch if same day
define get_next_version
$(shell \
	TODAY=$$(date +%Y%m%d); \
	LATEST=$$(git tag -l "v$$TODAY.*" | sort -V | tail -1); \
	if [ -z "$$LATEST" ]; then \
		echo "$$TODAY.0.0"; \
	else \
		PATCH=$$(echo "$$LATEST" | sed 's/.*\.0\.\([0-9]*\)/\1/'); \
		echo "$$TODAY.0.$$((PATCH + 1))"; \
	fi \
)
endef

build:
	cargo build --release

test:
	cargo test

clean:
	cargo clean

# Release: auto-version, update Cargo.toml, commit, tag, push
release:
	@VERSION=$(get_next_version); \
	echo "Releasing v$$VERSION"; \
	sed -i 's/^version = ".*"/version = "'$$VERSION'"/' Cargo.toml; \
	git add Cargo.toml; \
	git commit -m "Release v$$VERSION"; \
	git tag "v$$VERSION"; \
	git push origin main; \
	git push origin "v$$VERSION"
