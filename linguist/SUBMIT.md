# Getting "Tulpar" into GitHub's language statistics

## What does NOT work

`.gitattributes` with `*.tlp linguist-language=Tulpar` (already in this repo) is accepted by GitHub only if
"Tulpar" is a language that **Linguist already knows**. For an unknown name the override is ignored and the
`.tlp` files stay unclassified, so they do not appear in the language bar. This was checked with
`gh api repos/kuanysh228/tulpar/languages`. No file inside a repository can add a new language name.

## What works only for a known language

`.gitattributes` can relabel files as an *existing* Linguist language, e.g. `*.tlp linguist-language=Python`.
The files are then counted and shown under that other name, not as Tulpar.

## Official route (adding Tulpar to Linguist)

Linguist's own rules (CONTRIBUTING.md, "Language extension and filename usage requirements"):

* very new / hobby languages are **not accepted**: at least **2000 `.tlp` files** indexed on public GitHub in the
  last year (GitHub code search, forks excluded), spread over many unique `user/repo` combinations;
* a TextMate grammar in a **separate public repository** with a permissive license
  (`editors/vscode/syntaxes/tulpar.tmLanguage.json`, scope `source.tulpar`, is ready to be moved there);
* at least one (preferably two) real sample files in `samples/Tulpar/` — ready in `linguist/samples/Tulpar/`, MIT;
* an entry in `lib/linguist/languages.yml` — ready in `linguist/languages.yml.snippet` (omit `language_id`);
* then `script/add-grammar <grammar repo url>`, `script/update-ids`, `bundle exec rake test`, and a PR that links
  the code-search result.

Until the usage threshold is reached the PR will be closed, so the realistic plan is: make the repository public,
grow real-world usage, then submit.
