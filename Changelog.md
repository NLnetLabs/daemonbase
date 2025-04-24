# Change Log

## Unreleased next version

Breaking changes

New

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

