# virtual-switches

`vswitch` is a virtual Cisco switch in one binary: the IOS and IOS XE command line of Catalyst and industrial Ethernet switches, its commands compiled from Cisco's documentation for each platform and release and corrected by what real switches printed. It is for writing and testing switch configurations without hardware.

| Platform | Models | System | Releases |
|---|---|---|---|
| Catalyst 9300, 9300X, 9300L, 9300LM | 40 models, 14 network modules, stacks of 8 | IOS XE | 16.5, 16.6, 16.8 to 16.12, 17.1 to 17.18, 26.x |
| Catalyst 9200, 9200L | 23 models, 4 network modules, stacks of 8 | IOS XE | 16.9 to 16.12, 17.1 to 17.18, 26.x |
| Catalyst 9400 | C9404R, C9407R and C9410R chassis; 5 supervisors, 14 line cards | IOS XE | 16.6, 16.9 to 16.12, 17.1 to 17.18, 26.x |
| Catalyst 2960-X | 13 models, FlexStack-Plus stacks of 8 | IOS | 15.2(2)E, 15.2(4)E, 15.2(6)E, 15.2(7)E |
| Catalyst 2960, 2960-Plus, 2960-C | WS-C2960-24TC-L, WS-C2960G-24TC-L, WS-C2960G-8TC-L, WS-C2960+24TC-L, WS-C2960C-8TC-L | IOS | 12.2(44)SE, 12.2(55)SE, 15.0(2)SE, 15.2(2)E, 15.2(4)E |
| Catalyst IE3300 | IE-3300-8T2S | IOS XE | 17.6 |
| Cisco IE 3000 | IE-3000-8TC | IOS | 12.2(50)SE, 12.2(52)SE, 12.2(55)SE, 12.2(58)SE, 15.0(2)EY, 15.0(2)SE, 15.2(2)E |
| Cisco IE 3010 | IE-3010-16S-8PC | IOS | 12.2(53)EZ, 15.0(2)SE, 15.2(2)E |

Each platform book and release is one grammar in `data/grammars`, 93 in all, compiled into the binary. Written in Rust.

## Screens

The switch list, the hardware builder with its network module list open, a switch's console, and a kept configuration searched, at 100x30 in Windows Terminal's Campbell scheme. `cargo test --test terminal_audit screens -- --ignored` draws them again.

![The switch list](docs/screens/list.png)

![The hardware builder, its network module list open](docs/screens/builder.png)

![A switch's console](docs/screens/console.png)

![A kept configuration searched](docs/screens/configuration.png)

## Build and run

```
cargo build --release
target/release/vswitch
target/release/vswitch shell --stack C9300-24T --release 17.9
target/release/vswitch run tests/example-draft.txt
target/release/vswitch explain config-if "spanning-tree portfast trunk"
target/release/vswitch lab tests/example-lab.toml
```

## Commands

| Command | Does |
|---|---|
| (none) | the switch list; Enter opens a switch's console |
| `load` | adds a switch from a design or a configuration |
| `shell` | a console on a switch kept nowhere |
| `run` | types a script at its prompts and prints the session |
| `explain` | the command a line matches, its source and syntax |
| `examples` | types every example line of a release's books |
| `generate` | a configuration, a change script or a target configuration from a design file |
| `lint` | checks a configuration as it stands |
| `check` | the lines a release change breaks |
| `fleet` | every capture checked at its target release |
| `lab` | switches cabled together, each link checked from both ends |
| `topology` | a lab file of captured switches |
| `verify` | each line's evidence; `--outputs` the output rules no switch has shown |
| `capture` | a real switch read over SSH, a session log, or a folder of them |
| `changes` | what a backup changes on a captured switch |
| `probe` | a script typed on a lab switch and compared with vswitch |
| `sweep` | removes passwords and secrets from configurations and logs |
| `corpus` | compiles what captures teach into `data/corpus.json.gz` |
| `coverage` | the capture lines the books alone refuse |
| `releases` | the releases each book has a grammar for |
| `compile`, `compile-releases` | rebuild the grammars |

## Documentation

| Page | Covers |
|---|---|
| [Hardware](docs/hardware.md) | models, network modules, stacks, chassis, releases, port names, PoE |
| [Commands](docs/commands.md) | every command, its options and exit codes |
| [Terminal](docs/terminal.md) | the switch list, the console's keys and paging, the hardware builder |
| [Output](docs/output.md) | error messages, `?` help, privilege levels, each show command |
| [Checks](docs/checks.md) | lint, lab files and link checks, topology, design files |
| [Grammars](docs/grammars.md) | sources, compile rules, overrides, matching |
| [Captures](docs/captures.md) | the secret sweep, real switches, the corpus and crawls, capture, changes, probe |
| [Testing](docs/testing.md) | the test suite, CI, terminal tests |
