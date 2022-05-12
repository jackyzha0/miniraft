test:
	cargo test -- --color always --nocapture

test-debug:
	RUST_LOG=trace cargo test -- --test-threads 1 --color always --nocapture
