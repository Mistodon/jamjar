#[macro_use]
extern crate failure;
#[macro_use]
extern crate serde_derive;

extern crate handlebars;
extern crate image;
extern crate serde;
extern crate tempfile;
extern crate toml;
extern crate zip;

use std::io::Error as IOError;
use std::path::{Path, PathBuf};
use std::process::Command;

use image::ImageError;
use handlebars::{Handlebars, TemplateRenderError};
use toml::de::Error as TomlError;
use zip::{
    result::ZipError,
    write::{FileOptions, ZipWriter},
};

#[derive(Debug, Fail)]
pub enum JamjarError {
    #[fail(display = "an IO error occurred: {}", message)]
    IOError {
        #[cause]
        cause: IOError,
        message: String,
    },

    #[fail(
        display = "an error occurred while parsing TOML file: {}",
        cause
    )]
    TomlError {
        #[cause]
        cause: TomlError,
    },

    #[fail(
        display = "an error occurred while writing to template: {}",
        cause
    )]
    TemplateError {
        #[cause]
        cause: TemplateRenderError,
    },

    #[fail(display = "failed to decode icon image: {}", _0)]
    ImageError(ImageError),

    #[fail(display = "external command `{}` failed", _0)]
    ExternalCommandError(&'static str),

    #[fail(display = "an error occurred while compressing data: {}", _0)]
    ZipError(#[cause] ZipError),

    #[fail(display = "an error occurred: {}", _0)]
    StringError(String),
}

impl JamjarError {
    fn io(cause: IOError, message: &str) -> Self {
        JamjarError::IOError {
            cause,
            message: message.into(),
        }
    }
}

impl From<IOError> for JamjarError {
    fn from(cause: IOError) -> Self {
        let message = cause.to_string();
        JamjarError::IOError { cause, message }
    }
}

impl From<ZipError> for JamjarError {
    fn from(e: ZipError) -> Self {
        JamjarError::ZipError(e)
    }
}

impl From<ImageError> for JamjarError {
    fn from(e: ImageError) -> Self {
        JamjarError::ImageError(e)
    }
}

#[derive(Debug)]
pub struct Configuration {
    pub app_root: Option<PathBuf>,
    pub app_name: Option<String>,
    pub output_dir: PathBuf,
    pub icon_path: Option<PathBuf>,
}

struct AppConfig<'a> {
    app_root: &'a Path,
    app_name: &'a str,
    exe_name: &'a str,
    version: &'a str,
    bundle_id: &'a str,
    icon_path: &'a Path,
}

#[derive(Debug, Deserialize)]
struct CargoManifest {
    package: CargoManifestPackage,
}

#[derive(Debug, Deserialize)]
struct CargoManifestPackage {
    name: String,
    version: String,
}

pub fn package_app(config: &Configuration) -> Result<PathBuf, JamjarError> {
    use std::fs::File;

    let cwd = match config.app_root {
        Some(ref path) => path
            .canonicalize()
            .map_err(|e| JamjarError::io(e, &format!("The input directory '{}' could not be found.", path.display())))?,
        None => std::env::current_dir()
            .map_err(|e| JamjarError::io(e, "Failed to get current directory."))?,
    };

    println!("App is at: {}", cwd.display());

    println!("Compiling app for release:");
    {
        let output = Command::new("cargo")
            .current_dir(&cwd)
            .arg("build")
            .arg("--release")
            .output()?;

        print!("{}", String::from_utf8_lossy(&output.stdout));
        eprint!("{}", String::from_utf8_lossy(&output.stderr));

        if !output.status.success() {
            return Err(JamjarError::ExternalCommandError("cargo"));
        }
    }

    let manifest_toml = {
        let manifest_path = cwd.join("Cargo.toml");
        std::fs::read_to_string(&manifest_path)
            .map_err(|e| JamjarError::io(e, "Could not read Cargo.toml."))?
    };

    let manifest = toml::from_str::<CargoManifest>(&manifest_toml)
        .map_err(|e| JamjarError::TomlError { cause: e })?;

    let app_name = config.app_name.to_owned().unwrap_or(manifest.package.name.clone());
    let exe_name = manifest.package.name;

    let icon_path = match config.icon_path {
        Some(ref path) => path.to_owned(),
        None => cwd.join("icon.png"),
    };

    println!(
        "App name is: {}\nVersion is: {}\nIcon path is: {}",
        app_name, manifest.package.version, icon_path.display(),
    );

    std::fs::create_dir_all(&config.output_dir)
        .map_err(|e| JamjarError::io(e, "Failed to create output directory."))?;

    let output_path = config
        .output_dir
        .join(format!("{}_{}.zip", app_name, manifest.package.version));

    let temp_dir = tempfile::tempdir()
        .map_err(|e| JamjarError::io(e, "Failed to create temporary directory."))?;

    println!("Creating macOS app");

    let app_config = AppConfig {
        app_root: &cwd,
        app_name: &app_name,
        exe_name: &exe_name,
        version: &manifest.package.version,
        bundle_id: &app_name,
        icon_path: &icon_path,
    };

    let _app_path = create_macos_app(app_config, temp_dir.as_ref())?;

    println!("Compressing app to output");
    let mut output_file = File::create(&output_path)
        .map_err(|e| JamjarError::io(e, "Failed to create output file."))?;

    let mut zipper = ZipWriter::new(&mut output_file);
    let mut dirs = vec![temp_dir.as_ref().to_owned()];

    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(dir)? {
            use std::io::Write;

            let entry = entry?;
            let path = entry.path();

            if entry.file_type()?.is_file() {
                let rel_path = path.strip_prefix(&temp_dir).unwrap().to_owned();
                zipper.start_file(
                    rel_path.to_string_lossy(),
                    FileOptions::default().unix_permissions(0o755),
                )?;
                let contents = std::fs::read(path)?;
                zipper.write_all(&contents)?;
            } else {
                dirs.push(path);
            }
        }
    }

    zipper.finish()?;

    Ok(output_path)
}

fn create_macos_app(config: AppConfig, destination: &Path) -> Result<PathBuf, JamjarError> {
    use std::os::unix::fs::PermissionsExt;

    let AppConfig {
        app_root,
        app_name,
        exe_name,
        version,
        bundle_id,
        icon_path,
    } = config;

    let app_path = destination.join(format!("{}.app", app_name));
    let contents_path = app_path.join("Contents");
    let macos_path = contents_path.join("MacOS");
    let resources_path = contents_path.join("Resources");
    let plist_path = contents_path.join("Info.plist");
    let app_exe_path = macos_path.join(app_name);
    let app_icons_path = resources_path.join("Icon.icns");

    std::fs::create_dir_all(&macos_path)?;
    std::fs::create_dir_all(&resources_path)?;
    std::fs::create_dir_all(&contents_path)?;

    // Info.plist
    #[derive(Serialize)]
    struct InfoPlist<'a> {
        app_name: &'a str,
        version: &'a str,
        bundle_id: &'a str,
    }

    let template = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/templates/Info.plist"));
    let context = InfoPlist {
        app_name,
        version,
        bundle_id,
    };

    let hb = Handlebars::new();
    let info_plist = hb
        .render_template(&template, &context)
        .map_err(|e| JamjarError::TemplateError { cause: e })?;

    std::fs::write(&plist_path, &info_plist)
        .map_err(|e| JamjarError::io(e, "Failed to write Info.plist."))?;

    // Icons
    {
        println!("Creating icon set:");

        let temp_icons_dir = tempfile::tempdir()?;
        let temp_icons_dir = temp_icons_dir.as_ref().join(format!("{}.iconset", app_name));
        std::fs::create_dir(&temp_icons_dir)?;

        let image_bytes = std::fs::read(icon_path)?;
        let image = image::load_from_memory(&image_bytes)?;

        let sizes = &[
            ((16,16),    "icon_16x16.png"),
            ((32,32),    "icon_16x16@2x.png"),
            ((32,32),    "icon_32x32.png"),
            ((64,64),    "icon_32x32@2x.png"),
            ((128,128),  "icon_128x128.png"),
            ((256,256),  "icon_128x128@2x.png"),
            ((256,256),  "icon_256x256.png"),
            ((512,512),  "icon_256x256@2x.png"),
            ((512,512),  "icon_512x512.png"),
            ((1024,1024),"icon_512x512@2x.png"),
        ];

        for &((width, height), filename) in sizes {
            use image::FilterType;

            let resized_image = image.resize_exact(width, height, FilterType::CatmullRom);
            resized_image.save(temp_icons_dir.join(filename))?;
            println!("  Resized to {}", filename);
        }

        println!("Running iconutil");
        let output = Command::new("iconutil")
            .arg("-c")
            .arg("icns")
            .arg(temp_icons_dir)
            .arg("--output")
            .arg(&app_icons_path)
            .output()?;

        print!("{}", String::from_utf8_lossy(&output.stdout));
        eprint!("{}", String::from_utf8_lossy(&output.stderr));

        if !output.status.success() {
            return Err(JamjarError::ExternalCommandError("iconutil"));
        }
    }

    // Executable
    let exe_path = app_root.join(format!("target/release/{}", exe_name));
    std::fs::copy(&exe_path, &app_exe_path)?;

    let mut perms = std::fs::metadata(&app_exe_path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&app_exe_path, perms)?;

    Ok(app_path)
}
