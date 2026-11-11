# VOID Engine

Turn websites into lightweight desktop apps with Rust.

## Install

Install Rust and Cargo, then install VOID directly from the public repository:

```sh
cargo install --git https://github.com/nixhm55/VOID-ENGINE --bin void --locked
```

The first install builds the CLI for your computer. Cargo places the `void` command in its binary directory, usually `~/.cargo/bin`.

## Use

```sh
void --help
void https://example.com --name "VOID Test" --width 1000 --height 700
void init
```

`void init` creates or updates a `void.json` file in the current directory.
