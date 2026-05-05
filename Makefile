# Local servers for the D3 viewers.
# Usage:
#   make serve-app          # Rust: SQLite /api/graph + static files (default port 8080)
#   make serve              # python http.server only (no /api/graph)
#   make serve-viewer       # only viewer/ as doc root

.PHONY: serve serve-viewer serve-app

PORT ?= 8000

serve-app:
	@echo "SQLite-backed API + static files — http://localhost:$(PORT)/viewer/graph-explorer.html"
	@echo "  (set DATABASE_URL in .env; run link first)"
	cargo run -- serve --port $(PORT)

serve:
	@echo "Serving repo root on http://localhost:$(PORT)/"
	@echo "  Graph explorer: http://localhost:$(PORT)/viewer/graph-explorer.html"
	@echo "  (static JSON only: /out/graph.json, /data/findings/…; no /api/graph)"
	python3 -m http.server $(PORT)

serve-viewer:
	@echo "Serving repo root for viewer + artifacts on http://localhost:$(PORT)/"
	@echo "  Unified viewer: http://localhost:$(PORT)/viewer/index.html"
	@echo "  (enables /out/*.json access; no /api/graph)"
	python3 -m http.server $(PORT)
