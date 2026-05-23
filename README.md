# sitesaver
`sitesaver` is a CLI tool that fetches a web page, extracts the main readable content, and saves it as Markdown.
## Usage
```sh
sitesaver <URL>
```
## Options
```sh
sitesaver <URL> --output ./page.md
sitesaver <URL> --stdout
sitesaver <URL> --format markdown
sitesaver <URL> --mode main
```
## Development
```sh
cargo test
```
