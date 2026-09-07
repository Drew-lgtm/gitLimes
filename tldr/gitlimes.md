# gitlimes

> Ask a git repository what happened and who did it.
> Colored log, branches, author stats, and a unicode commit graph; memory stays flat no matter how long the history is.
> More information: <https://github.com/Drew-lgtm/gitLimes>.

- Show colored commit history for the current branch:

`gitlimes log`

- Show the last N commits:

`gitlimes log -n {{20}}`

- Show commits by a specific author since a date:

`gitlimes log --author={{name}} --since={{"2 weeks ago"}}`

- List branches with age, author and upstream tracking:

`gitlimes branches`

- List all branches, including remotes, and flag any untouched for N days:

`gitlimes branches -a --stale {{90}}`

- Show commit counts, an activity sparkline, and line counts per author:

`gitlimes who --lines`

- Draw a unicode commit graph across every branch:

`gitlimes graph --all -n {{60}}`

- Get machine-readable output from any command, as newline-delimited JSON:

`gitlimes {{subcommand}} --json`
