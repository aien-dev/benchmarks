#!/usr/bin/env python3
"""
AIEN Benchmarks: Like-for-Like Python Baseline Reference Microservice
Provides identical endpoints for direct apples-to-apples comparison against native Rust Axum services:
1. /health or /api/status -> Minimal JSON echo
2. /api/query -> SQLite record query with JSON serialization
3. /api/vector -> 768-dimensional vector dot product
"""
import http.server
import json
import sqlite3
import sys
import time

# Initialize in-memory SQLite table with sample entity
db = sqlite3.connect(":memory:", check_same_thread=False)
db.execute("CREATE TABLE entities (id INTEGER PRIMARY KEY, canonical_name TEXT, content TEXT)")
db.execute("INSERT INTO entities VALUES (1, 'benchmark_reference_entity', 'Verified cryptographic token record for baseline testing')")
db.commit()

class BaselineHandler(http.server.BaseHTTPRequestHandler):
    def log_message(self, format, *args):
        # Suppress logging to prevent IO latency pollution during benchmarking
        pass

    def do_GET(self):
        if self.path in ("/health", "/api/status"):
            body = json.dumps({"status": "ok", "engine": "python-baseline", "version": "3.12"}).encode("utf-8")
            status = 200
        elif self.path.startswith("/api/query"):
            cur = db.cursor()
            cur.execute("SELECT id, canonical_name, content FROM entities WHERE id = 1")
            row = cur.fetchone()
            body = json.dumps({"id": row[0], "canonical_name": row[1], "content": row[2]}).encode("utf-8")
            status = 200
        elif self.path.startswith("/api/vector"):
            # 768-dimensional float dot product
            v1 = [0.035] * 768
            v2 = [0.042] * 768
            dot = sum(x * y for x, y in zip(v1, v2))
            body = json.dumps({"similarity": dot, "dimensions": 768}).encode("utf-8")
            status = 200
        else:
            body = b'{"error": "not found"}'
            status = 404

        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)

if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 18191
    server = http.server.ThreadingHTTPServer(("127.0.0.1", port), BaselineHandler)
    print(f"PYTHON_BASELINE_READY:{port}", flush=True)
    server.serve_forever()
