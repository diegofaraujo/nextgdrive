//! Embeds `data/` (the app icon) as a GResource; see `data/resources.gresource.xml`.

fn main() {
    glib_build_tools::compile_resources(
        &["../../data"],
        "../../data/resources.gresource.xml",
        "nextgdrive.gresource",
    );
}
