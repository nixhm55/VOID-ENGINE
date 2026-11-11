# VOID Engine

VOID turns websites into lightweight native desktop and Android apps. The native app builds remain in the main `void` package; the separate `void-cli` package requests a branded cloud build and does not compile the app on the user's computer.

## Install the cloud client

Download the matching `void-macos`, `void-windows.exe`, or `void-linux` executable from a published GitHub Release and place it somewhere on your `PATH`. The `Release VOID Cloud Client` workflow attaches these clients to each published release.

Set `VOID_GITHUB_TOKEN` in your environment using a secret manager or a hidden-input prompt. The client sends build requests through the Vercel proxy; the token is used only to poll GitHub Actions and download artifacts, so it needs **Actions: read** access to the build repository. Keep it out of shell history and source files.

## Request a build

```sh
void https://example.com --name "My App"
void https://example.com --name "My App" --icon-url https://example.com/icon.png
```

The client sends the URL, app name, and optional icon URL to the Vercel build proxy, waits for that specific GitHub Actions run to finish, and downloads its artifact ZIP files into a new `void-build-<name>-<run-id>` folder in your current directory. The workflow builds the platform packages in parallel. Hosted runner queue times depend on GitHub capacity and cannot be guaranteed by the client.

The repository used for polling can be changed with `VOID_GITHUB_REPOSITORY`. See `void --help` for details.

## Build locally for development

The native wrapper can still be developed with Cargo from this repository. Its native runtime features and Android library are separate from the cloud client dependencies.
