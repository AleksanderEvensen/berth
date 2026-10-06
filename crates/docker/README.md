# Engine live views (M3)

Both implementations expose the unchanged `Engine` logs, stats and exec
contract. Streams and futures run on any executor. Drop a view's stream when
hiding it: its worker and timers stop, including all attached exec pumps.
Dropping exec stdin sends EOF; remote exec EOF also stops the other pumps.

`FixtureEngine::default().with_log_history_lines(1_000_000)` enables the UI
log performance fixture. Request `LogOptions { follow: false, tail: None,
since: None }` to consume it all. History is generated lazily and deterministically
from the container ID, image, creation time and line number. Default history is
240 lines; `LogOptions::default()` requests the last 1,000 and follows.
`with_latency` delays mutation results and log/stats/exec startup.

Fixture follow emits bursts and quiet periods while running, waits while paused,
and ends when stopped. Stats emit immediately after startup, then approximately
once per second, ending on pause/stop. The fake shell supports `ls`, `pwd`,
`whoami`, `echo`, `exit`, `stty size`, and `tput cols`, with CRLF, echo and backspace.
The configured working directory and user appear in its colored prompt.

Channel capacities are 1,024 log lines, 4 stats samples, and 64 exec chunks in
each direction. Futures mpsc also reserves one slot per sender. Resize uses a
single latest-value cell, coalesces bursts and skips duplicate sizes. Fixture
events have capacity 64; overflow closes the subscription so consumers resync.
No subscription means no streaming worker or timer.

Log timestamps are Unix nanoseconds (`Some` for fixtures), without the wire
prefix in `line`; text excludes LF/CRLF and retains ANSI sequences. Live timestamps
are `None` for missing/invalid prefixes. Live logs assemble stdout and stderr
independently, emitting complete lines in frame arrival order and flushing
remaining fragments at EOF. TTY/Console output is reported as stdout.
Stats timestamps come from Docker's `read`. CPU uses Docker CLI delta semantics;
missing previous values or decreasing CPU/system counters produce zero. Memory
excludes inactive-file cache, with cgroup v1 and v2 support. Byte sums saturate.

Bollard's generated log query represents `since` in i32 seconds. Out-of-range
bounds request a conservative representable bound and filter stamped output
locally, avoiding narrowing casts. Exec sets `ConsoleSize` at creation and resizes
immediately after attachment. Later resize errors are delivered through stdout.

Run `cargo test -p berth-docker -- --ignored` for Docker integration coverage.
The M3 tests create only unique Alpine containers and install cleanup before
creation. The cancellation test forwards a Unix Docker socket through a local
TCP proxy and verifies request EOF while the container and engine remain alive
(it is Unix-only and skips TCP/named-pipe endpoints). Existing storage integration
coverage creates its own disposable volume; no integration test prunes user images.
