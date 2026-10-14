# specifications-BASE

openEHR Base Model Component: the openEHR specification sources for the **BASE** component.

- Published specifications: https://specifications.openehr.org/releases/BASE/
- Layout, build and commit conventions: [AGENTS.md](AGENTS.md)
- Licence: Creative Commons Attribution-ShareAlike 3.0 Unported, see [LICENSE](LICENSE)

To render a local HTML preview, run this from the directory that holds the `specifications-*` clones (Docker is the only prerequisite):

```bash
docker run --rm -u $(id -u):$(id -g) -v "$PWD:/documents/" ghcr.io/openehr/asciidoctor development BASE
```
