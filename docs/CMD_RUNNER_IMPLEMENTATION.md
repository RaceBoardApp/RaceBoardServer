# Raceboard Command Runner

This document describes the `raceboard-cmd` adapter, a command-line tool for executing and tracking shell commands as races in the Raceboard server.

## Overview

The `raceboard-cmd` adapter is a key component of the Raceboard ecosystem. It allows you to track any shell command, from a simple `ls -la` to a complex build script, as a race in the Raceboard UI.

## Usage

To use the `raceboard-cmd` adapter, you simply prepend it to the command you want to track:

```bash
raceboard-cmd -- <command> [args...]
```

### Examples

**Track a simple command:**

```bash
raceboard-cmd -- ls -la
```

**Track a long-running process with a custom title:**

```bash
raceboard-cmd -t "Build Project" -- cargo build
```

**Track a command and show its output in the terminal:**

```bash
raceboard-cmd -o -- npm install
```

## Options

| Option | Short | Description |
| --- | --- | --- |
| `--title` | `-t` | Set a custom title for the race. |
| `--server` | `-s` | Specify the URL of the Raceboard server. |
| `--eta` | `-e` | Provide an estimated time in seconds for the race. |
| `--working-dir` | `-d` | Set the working directory for the command. |
| `--output` | `-o` | Show the command's output in the terminal. |
| `--metadata` | `-m` | Add key-value metadata to the race. |
| `--deeplink` | `-l` | Add a deeplink URL to the race. |

## Race Lifecycle

1.  **Created:** When you run `raceboard-cmd`, it first creates a new race in the Raceboard server with a `queued` status.
2.  **Running:** The adapter then executes the specified command and updates the race status to `running`.
3.  **Progress Updates:** If you provide an ETA, the adapter will automatically update the race's progress.
4.  **Output Events:** The adapter captures the command's output and sends it to the server as events.
5.  **Completion:** When the command finishes, the adapter updates the race status to `passed` or `failed` based on the command's exit code.
