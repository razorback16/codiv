---
title: Commands
description: Running shell commands in Command mode.
section: Usage
order: 1
---

## Command Mode

Codiv starts in AI mode by default. To run shell commands, switch to Command mode by pressing **Tab** on an empty input line. The gutter changes from `>` (cyan) to `$` (white), indicating that all input now goes directly to the bash co-process.

While in Command mode, everything you type is sent directly to the bash co-process. What you type is what runs.

## Running Commands

Type a command and press **Enter**. The command executes in the bash co-process and output appears inline in the terminal, just like a regular shell:

```
$ ls -la
total 48
drwxr-xr-x  12 user  staff   384 Mar 15 10:00 .
drwxr-xr-x   5 user  staff   160 Mar 15 09:00 ..
-rw-r--r--   1 user  staff  1024 Mar 15 10:00 Cargo.toml
...
```

Commands run sequentially. The prompt returns after each command completes.

## Interactive Passthrough

Programs that take over the full terminal (like `vim`, `ssh`, `python`, `top`) are detected and handed off to the PTY in passthrough mode. Codiv steps aside and lets the program control the terminal directly until it exits.

## Tab Completion

Bash tab completion works in Command mode. Press **Tab** while typing to complete file paths, command names, and other arguments, just as you would in a standard shell.

## Switching Back

To return to AI mode, press **Tab** on an empty input line. The gutter switches back to `>` (cyan) and input routes to the AI agent again.
