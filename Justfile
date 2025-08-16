@build:
    cargo build

@run:
    cargo run

@test:
    cargo test -- --nocapture

@clean:
    cargo clean
    find src/proto -type f -name '*.rs' ! -name 'mod.rs' -exec rm {} \+
