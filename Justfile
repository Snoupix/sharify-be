# https://just.systems/man/en/

@fmt:
    -cargo fmt

@build: fmt
    cargo build
    cp -R proto_ts/* ../sharify/src/lib/proto

@run *flags:
    cargo run {{flags}}

# This is needed to gracefully shutdown actix, a CTRL-C (or SIGINT) will force shutdown
[doc]
@stop:
    kill -s TERM $(pgrep sharify-be)

@update *flags:
    cargo update {{flags}}

@test:
    RUST_BACKTRACE=1 cargo test -- --nocapture

@dbg: build
    rust-gdb target/debug/sharify-be

@clean_proto:
    find src/proto -maxdepth 1 -type f -name '*.rs' ! -name 'mod.rs' -exec rm {} \+
    rm -rf proto_ts/* ../sharify/src/lib/proto/*

@clean: clean_proto
    cargo clean
