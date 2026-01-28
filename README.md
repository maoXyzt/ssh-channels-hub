# SSH Channels Hub

An CLI application to create and manage SSH channels.

Cross-platform (Windows, Linux), written in Rust.

## Features

- Read local configuration file to get the list of channels to open.
- Open SSH channels to remote servers according to the configuration file.
- If connection is lost, try to reconnect.
- If channel is closed, try to re-open it.
