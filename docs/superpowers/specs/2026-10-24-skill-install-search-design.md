# Reliable skill installation and search

The reported obra/superpowers import uses one global busy flag for every preview and removes every candidate after one success. Local diagnostics show the recorded install succeeded and its Personal SKILL.md exists. These facts explain misleading feedback; they do not establish every reported attempt succeeded.

Keep installed skills first and creation controls absent. Instructions toggle open/closed while cached documents and edit drafts remain owned by context. A selected import has its own identity; other candidates are disabled during that operation but never labelled as installing. Success merges the backend result immediately into the installed catalog and removes only that candidate. Refresh failure cannot conceal a successful install.

Install all is an explicit backend batch request over the reviewed pinned sources, bounded to 64 unique selections. Capture scope/workspace once. Reuse repository commit/tree metadata across the batch, install sequentially, and return a result per source. Existing skills are never implicitly overwritten; failures preserve other successful installs and failed candidates can be retried. Single and batch installations share storage validation, staging and read-back verification before success. No scripts run and no new frontend permissions are needed.

Add installed-skill search through the backend using BM25 (positive IDF, k1=1.2, b=0.75) over names, titles, descriptions and SKILL.md text. Tokenize Unicode alphanumeric text, lowercase, and split identifier separators. Score each unique query term, rank positive matches deterministically by score then scoped ID, return empty-query scope catalog. Search errors and late results cannot replace another context or block other skills actions. Do not send skill content to remote services for search.

Chosen approach: extend the existing Rust skill store/importer and native bridge, rather than introduce background job persistence or an external search service. This keeps operation ownership and successful persistence authoritative while avoiding new infrastructure.
