# BorizonServer

<p align="center">
  <b>A Rust-powered Minecraft server, built on Horizon.</b><br>
  Bringing native performance and a Rust-based plugin ecosystem to Minecraft server development.
</p>

<p align="center">
  <a href="https://github.com/RochaProjects/BorizonServer">
    <img src="https://img.shields.io/badge/status-in%20development-orange?style=for-the-badge" alt="In Development">
  </a>
  <a href="https://www.rust-lang.org/">
    <img src="https://img.shields.io/badge/language-Rust-orange?style=for-the-badge&logo=rust&logoColor=white" alt="Rust">
  </a>
  <a href="https://github.com/horizon-rust/horizon">
    <img src="https://img.shields.io/badge/based%20on-Horizon-blue?style=for-the-badge" alt="Based on Horizon">
  </a>
</p>

---

## Overview

**BorizonServer** is a community-driven fork of [Horizon](https://github.com/horizon-rust/horizon), focused on Minecraft dedicated server development.

Built on Horizon's Rust-based Minecraft implementation, BorizonServer aims to provide a high-performance, extensible server platform while introducing a native plugin ecosystem written in Rust.

The project explores a different approach to Minecraft server development: instead of relying on the traditional Java plugin ecosystem, BorizonServer aims to make Rust the foundation for both the server runtime and its extensions.

BorizonServer is actively under development. Its architecture and features are evolving, and compatibility and stability may change as development progresses.

## Goals

- **Native Rust performance:** Build upon Horizon's Rust implementation to provide an efficient dedicated server.
- **Rust plugin ecosystem:** Enable developers to create server extensions using Rust rather than Java.
- **Dynamic plugin loading:** Support independently compiled plugins, reducing the need to rebuild the entire server for every extension.
- **Extensible architecture:** Provide an SDK and API designed for server-side customization.
- **Modern server development:** Explore a more native, modular and maintainable approach to Minecraft server infrastructure.

## Plugin System

One of the main goals of BorizonServer is to introduce a native plugin system built around Rust.

The intention is to allow developers to build plugins as independent shared libraries, which can be loaded by the server at runtime.

This approach aims to provide:

- **Rust-native development:** Write server plugins using Rust and the BorizonServer SDK.
- **Independent compilation:** Build plugins separately from the server itself.
- **Dynamic loading:** Load supported plugins without recompiling the entire server.
- **Direct integration:** Interact with server functionality through a dedicated API.
- **Modular extensions:** Develop and maintain server features independently.

The plugin ecosystem and SDK are under development. APIs, interfaces and compatibility guarantees may change.

## Built on Horizon

BorizonServer builds upon [Horizon](https://github.com/horizon-rust/horizon), an open-source project that reimplements Minecraft: Java Edition in Rust.

Rather than developing a Minecraft server implementation from scratch, this project uses Horizon as its foundation and focuses on extending its server-side capabilities.

This means BorizonServer benefits from the underlying work of the Horizon project while pursuing its own direction for server architecture, extensibility and plugin development.

**BorizonServer is not affiliated with or endorsed by the Horizon development team.** It is an independent community fork.

## Project Status

BorizonServer is currently in active development.

The project is not yet intended to be considered a production-ready replacement for established Minecraft server software.

Features, APIs and implementation details are subject to change. Documentation and compatibility guarantees will evolve alongside development.

Contributions, testing and feedback are welcome.

## Development

BorizonServer is primarily developed in Rust.

For now, development should be based on the repository's source code and existing project structure. Build instructions, SDK documentation and plugin development guides will be expanded as the project matures.

- **Source code:** [Browse the repository](https://github.com/RochaProjects/BorizonServer)
- **Upstream project:** [Horizon](https://github.com/horizon-rust/horizon)
- **Issues:** [Report bugs or suggest features](https://github.com/RochaProjects/BorizonServer/issues)

## Acknowledgements

BorizonServer would not exist without the work of the [Horizon team](https://github.com/horizon-rust/horizon) and its contributors.

This project builds upon their work and aims to extend it in a different direction, focusing on dedicated server infrastructure and a native Rust plugin ecosystem.

We encourage users and developers to explore the original Horizon project as well.

## License

BorizonServer is based on Horizon. Please refer to the repository's `LICENSE` file and applicable third-party license notices for licensing information.

## Disclaimer

**NOT AN OFFICIAL MINECRAFT PRODUCT. NOT APPROVED BY OR ASSOCIATED WITH MOJANG OR MICROSOFT.**

Minecraft is a trademark of Mojang Synergies AB. BorizonServer is an independent community project and is not affiliated with Mojang, Microsoft or the Horizon development team.
