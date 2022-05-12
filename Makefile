test:
	cargo test -- --color always

test-debug:
	RUST_LOG=trace cargo test -- --test-threads 1 --color always
