
use std::{env, fmt, fs, ops};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use log::error;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use toml_edit as toml;
use crate::error::Failed;


//------------ ConfigFile ----------------------------------------------------

/// The content of a config file.
///
/// This is a thin wrapper around `toml::Table` to make dealing with it more
/// convenient.
#[derive(Clone, Debug)]
pub struct ConfigFile {
    /// The content of the file.
    content: toml::DocumentMut,

    /// The path to the config file.
    path: PathBuf,

    /// The directory we found the file in.
    ///
    /// This is used in relative paths.
    dir: PathBuf,
}

impl ConfigFile {
    /// Reads the config file at the given path.
    ///
    /// If there is no such file, returns `None`. If there is a file but it
    /// is broken, aborts.
    #[allow(clippy::verbose_file_reads)]
    pub fn read(path: &Path) -> Result<Option<Self>, Failed> {
        let mut file = match fs::File::open(path) {
            Ok(file) => file,
            Err(_) => return Ok(None)
        };
        let mut config = String::new();
        if let Err(err) = file.read_to_string(&mut config) {
            error!(
                "Failed to read config file {}: {}",
                path.display(), err
            );
            return Err(Failed);
        }
        Self::parse(&config, path).map(Some)
    }

    /// Parses the content of the file from a string.
    pub fn parse(content: &str, path: &Path) -> Result<Self, Failed> {
        let content = match toml::DocumentMut::from_str(content) {
            Ok(content) => content,
            Err(err) => {
                eprintln!(
                    "Failed to parse config file {}: {}",
                    path.display(), err
                );
                return Err(Failed);
            }
        };
        let dir = if path.is_relative() {
            path.join(match env::current_dir() {
                Ok(dir) => dir,
                Err(err) => {
                    error!(
                        "Fatal: Can't determine current directory: {err}.",
                    );
                    return Err(Failed);
                }
            }).parent().unwrap().into() // a file always has a parent
        }
        else {
            path.parent().unwrap().into()
        };
        Ok(ConfigFile {
            content,
            path: path.into(),
            dir
        })
    }

    /// Returns a reference to the path of the config file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Takes a value from the from the config file if present.
    pub fn take_value(
        &mut self, key: &str
    ) -> Result<Option<toml::Value>, Failed> {
        match self.content.remove(key) {
            Some(toml::Item::Value(value)) => Ok(Some(value)),
            Some(_) => {
                error!(
                    "Failed in config file {}: \
                     '{}' expected to be a value.",
                    self.path.display(), key
                );
                Err(Failed)
            }
            None => Ok(None)
        }
    }
    

    /// Takes a boolean value from the config file.
    ///
    /// The value is taken from the given `key`. Returns `Ok(None)` if there
    /// is no such key. Returns an error if the key exists but the value
    /// isn’t a booelan.
    pub fn take_bool(&mut self, key: &str) -> Result<Option<bool>, Failed> {
        match self.take_value(key)? {
            Some(toml::Value::Boolean(res)) => Ok(Some(res.into_value())),
            Some(_) => {
                error!(
                    "Failed in config file {}: \
                     '{}' expected to be a boolean.",
                    self.path.display(), key
                );
                Err(Failed)
            }
            None => Ok(None)
        }
    }

    /// Takes an unsigned integer value from the config file.
    ///
    /// The value is taken from the given `key`. Returns `Ok(None)` if there
    /// is no such key. Returns an error if the key exists but the value
    /// isn’t an integer or if it is negative.
    pub fn take_u64(&mut self, key: &str) -> Result<Option<u64>, Failed> {
        match self.take_value(key)? {
            Some(toml::Value::Integer(value)) => {
                match u64::try_from(value.into_value()) {
                    Ok(value) => Ok(Some(value)),
                    Err(_) => {
                        error!(
                            "Failed in config file {}: \
                            '{}' expected to be a positive integer.",
                            self.path.display(), key
                        );
                        Err(Failed)
                    }
                }
            }
            Some(_) => {
                error!(
                    "Failed in config file {}: \
                     '{}' expected to be an integer.",
                    self.path.display(), key
                );
                Err(Failed)
            }
            None => Ok(None)
        }
    }

    /// Takes a limited unsigned 8-bit integer value from the config file.
    ///
    /// The value is taken from the given `key`. Returns `Ok(None)` if there
    /// is no such key. Returns an error if the key exists but the value
    /// isn’t an integer, is larger than `limit` or is negative.
    pub fn take_limited_u8(
        &mut self, key: &str, limit: u8,
    ) -> Result<Option<u8>, Failed> {
        match self.take_u64(key)? {
            Some(value) => {
                match u8::try_from(value) {
                    Ok(value) => {
                        if value > limit {
                            error!(
                                "Failed in config file {}: \
                                '{}' expected integer between 0 and {}.",
                                self.path.display(), key, limit,
                            );
                            Err(Failed)
                        }
                        else {
                            Ok(Some(value))
                        }
                    }
                    Err(_) => {
                        error!(
                            "Failed in config file {}: \
                            '{}' expected integer between 0 and {}.",
                            self.path.display(), key, limit,
                        );
                        Err(Failed)
                    }
                }
            }
            None => Ok(None)
        }
    }

    /// Takes an unsigned integer value from the config file.
    ///
    /// The value is taken from the given `key`. Returns `Ok(None)` if there
    /// is no such key. Returns an error if the key exists but the value
    /// isn’t an integer or if it is negative.
    pub fn take_usize(&mut self, key: &str) -> Result<Option<usize>, Failed> {
        match self.take_u64(key)? {
            Some(value) => {
                match usize::try_from(value) {
                    Ok(value) => Ok(Some(value)),
                    Err(_) => {
                        error!(
                            "Failed in config file {}: \
                            '{}' expected to be a positive integer.",
                            self.path.display(), key
                        );
                        Err(Failed)
                    }
                }
            }
            None => Ok(None)
        }
    }

    /// Takes a small unsigned integer value from the config file.
    ///
    /// While the result is returned as an `usize`, it must be in the
    /// range of a `u16`.
    ///
    /// The value is taken from the given `key`. Returns `Ok(None)` if there
    /// is no such key. Returns an error if the key exists but the value
    /// isn’t an integer or if it is out of bounds.
    pub fn take_small_usize(
        &mut self, key: &str
    ) -> Result<Option<usize>, Failed> {
        match self.take_usize(key)? {
            Some(value) => {
                if value > u16::MAX.into() {
                    error!(
                        "Failed in config file {}: \
                        value for '{}' is too large.",
                        self.path.display(), key
                    );
                    Err(Failed)
                }
                else {
                    Ok(Some(value))
                }
            }
            None => Ok(None)
        }
    }

    /// Takes a string value from the config file.
    ///
    /// The value is taken from the given `key`. Returns `Ok(None)` if there
    /// is no such key. Returns an error if the key exists but the value
    /// isn’t a string.
    pub fn take_string(
        &mut self, key: &str
    ) -> Result<Option<String>, Failed> {
        match self.take_value(key)? {
            Some(toml::Value::String(value)) => {
                Ok(Some(value.into_value()))
            }
            Some(_) => {
                error!(
                    "Failed in config file {}: \
                     '{}' expected to be a string.",
                    self.path.display(), key
                );
                Err(Failed)
            }
            None => Ok(None)
        }
    }

    /// Takes a string encoded value from the config file.
    ///
    /// The value is taken from the given `key`. It is expected to be a
    /// string and will be converted to the final type via `FromStr::from_str`.
    ///
    /// Returns `Ok(None)` if the key doesn’t exist. Returns an error if the
    /// key exists but the value isn’t a string or conversion fails.
    pub fn take_from_str<T>(&mut self, key: &str) -> Result<Option<T>, Failed>
    where T: FromStr, T::Err: fmt::Display {
        match self.take_string(key)? {
            Some(value) => {
                match T::from_str(&value) {
                    Ok(some) => Ok(Some(some)),
                    Err(err) => {
                        error!(
                            "Failed in config file {}: \
                             illegal value in '{}': {}.",
                            self.path.display(), key, err
                        );
                        Err(Failed)
                    }
                }
            }
            None => Ok(None)
        }
    }

    /// Takes a path value from the config file.
    ///
    /// The path is taken from the given `key`. It must be a string value.
    /// It is treated as relative to the directory of the config file. If it
    /// is indeed a relative path, it is expanded accordingly and an absolute
    /// path is returned.
    ///
    /// Returns `Ok(None)` if the key does not exist. Returns an error if the
    /// key exists but the value isn’t a string.
    pub fn take_path(
        &mut self, key: &str
    ) -> Result<Option<ConfigPath>, Failed> {
        self.take_string(key).map(|opt| {
            opt.map(|path| self.dir.join(path).into())
        })
    }

    /// Takes a mandatory path value from the config file.
    ///
    /// This is the pretty much the same as [`take_path`] but also returns
    /// an error if the key does not exist.
    ///
    /// [`take_path`]: #method.take_path
    pub fn take_mandatory_path(
        &mut self, key: &str
    ) -> Result<ConfigPath, Failed> {
        match self.take_path(key)? {
            Some(res) => Ok(res),
            None => {
                error!(
                    "Failed in config file {}: missing required '{}'.",
                    self.path.display(), key
                );
                Err(Failed)
            }
        }
    }

    /// Takes an array of strings from the config file.
    ///
    /// The value is taken from the entry with the given `key` and, if
    /// present, the entry is removed. The value must be an array of strings.
    /// If the key is not present, returns `Ok(None)`. If the entry is present
    /// but not an array of strings, returns an error.
    pub fn take_string_array(
        &mut self,
        key: &str
    ) -> Result<Option<Vec<String>>, Failed> {
        match self.take_value(key)? {
            Some(toml::Value::Array(vec)) => {
                let mut res = Vec::new();
                for value in vec.into_iter() {
                    if let toml::Value::String(value) = value {
                        res.push(value.into_value())
                    }
                    else {
                        error!(
                            "Failed in config file {}: \
                            '{}' expected to be a array of strings.",
                            self.path.display(),
                            key
                        );
                        return Err(Failed);
                    }
                }
                Ok(Some(res))
            }
            Some(_) => {
                error!(
                    "Failed in config file {}: \
                     '{}' expected to be a array of strings.",
                    self.path.display(), key
                );
                Err(Failed)
            }
            None => Ok(None)
        }
    }

    /// Takes an array of string encoded values from the config file.
    ///
    /// The value is taken from the entry with the given `key` and, if
    /// present, the entry is removed. The value must be an array of strings.
    /// Each string is converted to the output type via `FromStr::from_str`.
    ///
    /// If the key is not present, returns `Ok(None)`. If the entry is present
    /// but not an array of strings or if converting any of the strings fails,
    /// returns an error.
    pub fn take_from_str_array<T>(
        &mut self,
        key: &str
    ) -> Result<Option<Vec<T>>, Failed>
    where T: FromStr, T::Err: fmt::Display {
        match self.take_value(key)? {
            Some(toml::Value::Array(vec)) => {
                let mut res = Vec::new();
                for value in vec.into_iter() {
                    if let toml::Value::String(value) = value {
                        match T::from_str(value.value()) {
                            Ok(value) => res.push(value),
                            Err(err) => {
                                error!(
                                    "Failed in config file {}: \
                                     Invalid value in '{}': {}",
                                    self.path.display(), key, err
                                );
                                return Err(Failed)
                            }
                        }
                    }
                    else {
                        error!(
                            "Failed in config file {}: \
                            '{}' expected to be a array of strings.",
                            self.path.display(),
                            key
                        );
                        return Err(Failed)
                    }
                }
                Ok(Some(res))
            }
            Some(_) => {
                error!(
                    "Failed in config file {}: \
                     '{}' expected to be a array of strings.",
                    self.path.display(), key
                );
                Err(Failed)
            }
            None => Ok(None)
        }
    }

    /// Takes an array of paths from the config file.
    ///
    /// The values are taken from the given `key` which must be an array of
    /// strings. Each path is treated as relative to the directory of the
    /// config file. All paths are expanded if necessary and are returned as
    /// absolute paths.
    ///
    /// Returns `Ok(None)` if the key does not exist. Returns an error if the
    /// key exists but the value isn’t an array of string.
    pub fn take_path_array(
        &mut self,
        key: &str
    ) -> Result<Option<Vec<ConfigPath>>, Failed> {
        match self.take_value(key)? {
            Some(toml::Value::String(value)) => {
                Ok(Some(vec![self.dir.join(value.into_value()).into()]))
            }
            Some(toml::Value::Array(vec)) => {
                let mut res = Vec::new();
                for value in vec.into_iter() {
                    if let toml::Value::String(value) = value {
                        res.push(self.dir.join(value.into_value()).into())
                    }
                    else {
                        error!(
                            "Failed in config file {}: \
                            '{}' expected to be a array of paths.",
                            self.path.display(),
                            key
                        );
                        return Err(Failed);
                    }
                }
                Ok(Some(res))
            }
            Some(_) => {
                error!(
                    "Failed in config file {}: \
                     '{}' expected to be a array of paths.",
                    self.path.display(), key
                );
                Err(Failed)
            }
            None => Ok(None)
        }
    }

    /// Takes a string-to-string hashmap from the config file.
    pub fn take_string_map(
        &mut self,
        key: &str
    ) -> Result<Option<HashMap<String, String>>, Failed> {
        match self.take_value(key)? {
            Some(toml::Value::Array(vec)) => {
                let mut res = HashMap::new();
                for value in vec.into_iter() {
                    let mut pair = match value {
                        toml::Value::Array(pair) => pair.into_iter(),
                        _ => {
                            error!(
                                "Failed in config file {}: \
                                '{}' expected to be a array of string pairs.",
                                self.path.display(),
                                key
                            );
                            return Err(Failed);
                        }
                    };
                    let left = match pair.next() {
                        Some(toml::Value::String(value)) => value,
                        _ => {
                            error!(
                                "Failed in config file {}: \
                                '{}' expected to be a array of string pairs.",
                                self.path.display(),
                                key
                            );
                            return Err(Failed);
                        }
                    };
                    let right = match pair.next() {
                        Some(toml::Value::String(value)) => value,
                        _ => {
                            error!(
                                "Failed in config file {}: \
                                '{}' expected to be a array of string pairs.",
                                self.path.display(),
                                key
                            );
                            return Err(Failed);
                        }
                    };
                    if pair.next().is_some() {
                        error!(
                            "Failed in config file {}: \
                            '{}' expected to be a array of string pairs.",
                            self.path.display(),
                            key
                        );
                        return Err(Failed);
                    }
                    if res.insert(
                        left.into_value(), right.into_value()
                    ).is_some() {
                        error!(
                            "Failed in config file {}: \
                            'duplicate item in '{}'.",
                            self.path.display(),
                            key
                        );
                        return Err(Failed);
                    }
                }
                Ok(Some(res))
            }
            Some(_) => {
                error!(
                    "Failed in config file {}: \
                     '{}' expected to be a array of string pairs.",
                    self.path.display(), key
                );
                Err(Failed)
            }
            None => Ok(None)
        }
    }

    /// Checks whether the config file is now empty.
    ///
    /// If it isn’t, logs a complaint and returns an error.
    pub fn check_exhausted(&self) -> Result<(), Failed> {
        if !self.content.is_empty() {
            print!(
                "Failed in config file {}: Unknown settings ",
                self.path.display()
            );
            let mut first = true;
            for (key, _) in self.content.iter() {
                if !first {
                    print!(",");
                }
                else {
                    first = false
                }
                print!("{key}");
            }
            error!(".");
            Err(Failed)
        }
        else {
            Ok(())
        }
    }

    /// Inserts a string value.
    pub fn insert_string(&mut self, key: &str, value: impl ToString) {
        self.content.insert(key, toml::Item::Value(
            toml::Value::String(
                toml::Formatted::new(
                    value.to_string()
                )
            )
        ));
    }

    /// Insert a path value.
    pub fn insert_path(&mut self, key: &str, path: &Path) {
        let path = match path.strip_prefix(&self.dir) {
            Ok(path) => path,
            Err(_) => path
        };
        self.insert_string(key, path.display())
    }
}


//------------ ConfigPath ----------------------------------------------------

/// A path encountered in a config file.
///
/// This is a basically a `PathBuf` that, when, deserialized or used as a
/// command line argument resolves all relative paths into absolute paths.
///
/// When used as a command line argument with clap, it will use the current
/// working directory as the base path.
///
/// When used with serde, the base path can be through
/// [`set_base_path`][Self::set_base_path] before deserialization or
/// serialization and cleared afterwards with
/// [`clear_base_path`.][Self::clear_base_path].
///
/// Under the hood, this uses a thread local variable, so (de-) serializers
/// that somehow spawn threads may not work as expected.
#[derive(
    Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd,
)]
pub struct ConfigPath(PathBuf);

impl ConfigPath {
    thread_local!(
        static BASE_PATH: RefCell<Option<PathBuf>> = const {
            RefCell::new(None)
        }
    );

    pub fn set_base_path(path: PathBuf) {
        Self::BASE_PATH.with(|base_path| {
            base_path.replace(Some(path));
        })
    }

    pub fn clear_base_path() {
        Self::BASE_PATH.with(|base_path| {
            base_path.replace(None);
        })
    }

    fn construct(path: PathBuf) -> Self {
        Self::BASE_PATH.with(|base_path| {
            Self(
                match base_path.borrow().as_ref() {
                    Some(base_path) => base_path.join(&path),
                    None => path
                }
            )
        })
    }

    fn deconstruct(&self) -> &Path {
        Self::BASE_PATH.with(|base_path| {
            match base_path.borrow().as_ref() {
                Some(base_path) => {
                    match self.0.strip_prefix(base_path) {
                        Ok(path) => path,
                        Err(_) => self.0.as_ref(),
                    }
                }
                None => self.0.as_ref()
            }
        })
    }
}

impl ConfigPath {
    /// Returns the reference to the actual path.
    pub fn as_path(&self) -> &Path {
        self.0.as_ref()
    }
}

impl From<PathBuf> for ConfigPath {
    fn from(path: PathBuf) -> Self {
        Self::construct(path)
    }
}

impl From<String> for ConfigPath {
    fn from(path: String) -> Self {
        Self::construct(path.into())
    }
}

impl From<ConfigPath> for PathBuf {
    fn from(path: ConfigPath) -> Self {
        path.0
    }
}

impl ops::Deref for ConfigPath {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        self.0.as_ref()
    }
}

impl AsRef<Path> for ConfigPath {
    fn as_ref(&self) -> &Path {
        self.0.as_ref()
    }
}

impl<'de> Deserialize<'de> for ConfigPath {
    fn deserialize<D: Deserializer<'de>>(
        deserializer: D
    ) -> Result<Self, D::Error> {
        Ok(Self::construct(PathBuf::deserialize(deserializer)?))
    }
}

impl Serialize for ConfigPath {
    fn serialize<S: Serializer>(
        &self, serializer: S
    ) -> Result<S::Ok, S::Error> {
        self.deconstruct().serialize(serializer)
    }
}

impl clap::builder::ValueParserFactory for ConfigPath {
    type Parser = ConfigPathParser;

    fn value_parser() -> Self::Parser {
        ConfigPathParser
    }
}


//------------ ConfigPathParser ----------------------------------------------

#[derive(Clone)]
pub struct ConfigPathParser;

impl clap::builder::TypedValueParser for ConfigPathParser {
    type Value = ConfigPath;

    fn parse_ref(
        &self,
        cmd: &clap::Command,
        arg: Option<&clap::Arg>,
        value: &std::ffi::OsStr,
    ) -> Result<Self::Value, clap::Error> {
        let path = clap::builder::PathBufValueParser::new().parse_ref(
            cmd, arg, value
        )?;
        if path.is_absolute() {
            return Ok(ConfigPath(path));
        }
        let cur_dir = match env::current_dir() {
            Ok(dir) => dir,
            Err(err) => {
                let mut res = clap::Error::new(
                    clap::error::ErrorKind::Io
                ).with_cmd(cmd);
                res.insert(
                    clap::error::ContextKind::Custom,
                    clap::error::ContextValue::String(
                        format!("Failed to get current directory: {err}")
                    )
                );
                return Err(res);
            }
        };
        Ok(ConfigPath(cur_dir.join(path)))
    }
}

