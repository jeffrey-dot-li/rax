# LLaMa 7b in rust

[![](https://dcbadge.vercel.app/api/server/AtUhGqBDP5)](https://discord.gg/AtUhGqBDP5)

This repo contains the popular [LLaMa 7b](https://ai.facebook.com/blog/large-language-model-llama-meta-ai/)
language model, fully implemented in the rust programming language!

Uses [dfdx](https://github.com/coreylowman/dfdx) tensors and CUDA acceleration.

# How To Run

## (Once) Setting up model weights

### Download model weights
1. Install git lfs. On ubuntu you can run `sudo apt install git-lfs`
2. Activate git lfs with `git lfs install`.
3. Run `bash download.sh` to download the model weights in pytorch format (~25 GB)

### Convert the model
1. (Optional) Run `python3.x -m venv <my_env_name>` to create a python virtual environment, where `x` is your prefered python version
2. (Optional, requires 1.) Run `source <my_env_name>\bin\activate` (or `<my_env_name>\Scripts\activate` if on Windows) to activate the environment
3. Run `pip install numpy torch`
4. Run `python convert.py` to convert the model weights to rust understandable format

## (Once) Compile

You can compile with normal rust commands:

With cuda:
```bash
cargo build --release -F cuda
```

Without cuda:
```bash
cargo build --release
```

## Run the executable

With default args:
```bash
./target/release/llama-dfdx generate "<prompt>"
./target/release/llama-dfdx chat
./target/release/llama-dfdx file <path to prompt file>
```

To see what commands/custom args you can use:
```bash
./target/release/llama-dfdx --help
```
