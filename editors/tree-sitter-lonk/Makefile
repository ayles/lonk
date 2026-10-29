TREE_SITTER ?= tree-sitter
NVIM_RUNTIME ?= ../nvim

.PHONY: generate build test nvim
generate:
	$(TREE_SITTER) generate --js-runtime native
build:
	$(TREE_SITTER) build -o lonk.so
test:
	$(TREE_SITTER) test
	$(TREE_SITTER) parse -q ../../examples/*.lonk ../../crates/lonk/std/*.lonk
nvim: build
	mkdir -p "$(NVIM_RUNTIME)/parser" "$(NVIM_RUNTIME)/queries/lonk"
	cp lonk.so "$(NVIM_RUNTIME)/parser/lonk.so"
	cp queries/*.scm "$(NVIM_RUNTIME)/queries/lonk/"
