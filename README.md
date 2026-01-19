# Crater

Crater is a container isolation tool written in Rust that provides namespace isolation capabilities using Linux
namespaces.

## Description

Crater creates isolated environments by utilizing Linux namespace features, specifically UTS (Unix Time-sharing System)
and PID (Process ID) namespaces. This allows for better process isolation and container-like functionality.

## Requirements

- Linux operating system
- Root privileges
- Rust toolchain
- Docker (optional, for containerized usage)

## Installation

1. Clone the repository
2. Build the project:
