AGENT: never change this file! This purely for humans to edit!
What do we need:
- resolution of metadata? Which tools are available?
    - resolve the path of the tools (where is that actually)
    - 
- cli: so all the cli commands as one clap tool
    - hand over permissions to the tools, e.g. --read-dir, --write-dir, --write-file, and some http stuff
    - sat <toolname> --help
    - sat --help
    - I assume this should be the main entry point
    - should document how the cli can be used in the top comment
- python-venv
- runtime.rs
    - check that the tool that we run is valid wasi
    - check that it only requests the permissions as given by the wit file, so this needs to resolve the path to tools as well (but does so from the global and local config.tomls)
- main.rs: basically calls the cli!
- maybe some http handling  -> probably in runtime.rs
- where does read and write happen? -> probably in runtime.rs
- metadata.rs might be a good idea
- plugin_handler.rs: Registers plugins, adds their name and path to .sat/config.toml (can be enabled locally or globally) and checks whether plugin conforms to standars, so sat add plugin <pluginpath> --global or --local should be a good enough cli option!


- what permissions:
    - permissions are defined in some file .sat/config.toml, or in some global file ~/.sat/config.toml, local overwrites global
    - permissions can be restricted using the cli! So if in config.toml there is a permission set for the folders a tool can access, we can simply limit those. E.g. in config.toml it says [[read_dirs]] . (local_execution directory) and --read-dir ./tmp (just limits this to the local tmp dir instead of complete local file).
    - a global [[read_dirs]] [[write_dirs]], [[http]] etc. should be there, this is not overwritable by local grants and should give a simple version of what can be done! To look up permissions easily. These don't need to be set necessarily.
    - optional: cli command to check permissios for each tool. The permissios are then a combination of global configs and tool specific configs and can depend on the project where sat is currently used (so whether there is a local .sat/config.toml that overwrites global permissions). We need to check if the plugin name is already registered, if so then throw an error!
    - Where are secrets (API keys, etc.) stored? Or are they simply handed over to the tool via ${SECRET_NAME}? Is this safe enough? Or would we need to compose with another secrets tool that has access to a .env file and then extracts just one secret with the corresponding SECRET_NAME and hands that over via wasm? So should secrets always be part of the wit interface (I would think so, to make it explicit)!Actually exposing the secrets as OS variables is kind of unsafe, I am very sure about that (should check though). Maybe we could even build something like a keyvault using an extension, so that we insert the things once, only the sat keyvault tool has access (how to actually do that?) and one can extract that but it only hands over the secrets via a chaining commands to other sat tools, though there is no clear way to prevent them from printing to the cli if they have cli access or returning them in other ways, but I assume this is ok-ish, since this is the same trust boundary as for other tools (not sat) as well, or even a smaller trust boundary. It's probably easiest to just let pi (or sat) talk to a secret manager!

- plugin architecture:
    - plugins need to expose a certain method, e.g. run, also they need to have a wit file and a <toolname>.toml file with their permissions
    - once a plugin is registered in the config.toml there is an entry [[toolname]] path = <path_to_plugin>, cli_command = <cli_command_to_invoke after sat <command>
