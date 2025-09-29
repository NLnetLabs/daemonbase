# Change Log

## 0.1.4

Released 2025-09-29.

New

* Reworked setting of user and group. If not provided explicitly, the
  group will now be set to the user’s group. The list of supplemental
  groups will be initialized from the user’s group list. ([#11])
* Allow manually creating the process configuration so it can be used
  without the _clap_ argument parser. ([#12])
* Added support for systemd’s socket activation. ([#13])

[11]: https://github.com/NLnetLabs/daemonbase/pull/11
[12]: https://github.com/NLnetLabs/daemonbase/pull/12
[13]: https://github.com/NLnetLabs/daemonbase/pull/13


## 0.1.3

Released 2025-04-24.

Bug fixes

* The `working-dir` option was accidentally used as the path for the PID
  file. Now the `pid-file` option is used as intended. ([#7])

Other changes

* The minimum supported Rust version is now 1.81. ([#7])

[7]: https://github.com/NLnetLabs/daemonbase/pull/7


## 0.1.2

Released 2024-06-18.

Bug fixes

* Don’t overwrite the log level when no command line options for it are
  given. ([b2a2f58])

[b2a2f58]: https://github.com/NLnetLabs/daemonbase/commit/b2a2f58c53116df30fa6464e3c224fabb1f2dc3b


## 0.1.1

Released 2024-05-29.

Other changes

* Updated to `toml_edit` 0.22. ([#4])

[4]: https://github.com/NLnetLabs/daemonbase/pull/4


## 0.1.0

Released 2024-03-08.

Initial release.

