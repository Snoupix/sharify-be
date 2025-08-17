@build:
    cargo build

@run:
    cargo run

@test:
    cargo test -- --nocapture

@clean:
    cargo clean
    find src/proto -maxdepth 1 -type f -name '*.rs' ! -name 'mod.rs' -exec rm {} \+
