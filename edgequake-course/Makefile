.PHONY: build serve clean validate install-deps

# Build the mdbook course
build:
	mdbook build

# Serve locally with live reload
serve:
	mdbook serve --open

# Clean build artifacts
clean:
	mdbook clean

# Validate links and structure
validate:
	mdbook build 2>&1 | grep -E "(ERROR|WARNING)" || echo "No errors found"

# Install required mdbook preprocessors
install-deps:
	cargo install mdbook
	cargo install mdbook-mermaid
	cargo install mdbook-quiz
	cargo install mdbook-exercises
