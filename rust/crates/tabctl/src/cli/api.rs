use super::*;

pub fn run<I, T>(args: I) -> Result<(), String>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let matches = build_cli()
        .try_get_matches_from(args)
        .map_err(|e| e.to_string())?;
    if matches.get_flag("version") {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if let Some(("help", sub)) = matches.subcommand() {
        return run_help(&matches, sub);
    }
    if let Some(("version", sub)) = matches.subcommand() {
        return run_version(&matches, sub);
    }
    if let Some(("setup", sub)) = matches.subcommand() {
        return run_setup(&matches, sub);
    }
    if let Some(("extension-fetch", sub)) = matches.subcommand() {
        return run_extension_fetch(&matches, sub);
    }
    if let Some(("doctor", sub)) = matches.subcommand() {
        return run_doctor(&matches, sub);
    }
    if let Some(("policy", sub)) = matches.subcommand() {
        return run_policy(&matches, sub);
    }
    if let Some(("skill", sub)) = matches.subcommand() {
        return run_skill(&matches, sub);
    }
    if let Some(("profile-list", sub)) = matches.subcommand() {
        return run_profile_list(&matches, sub);
    }
    if let Some(("profile-show", sub)) = matches.subcommand() {
        return run_profile_show(&matches, sub);
    }
    if let Some(("profile-switch", sub)) = matches.subcommand() {
        return run_profile_switch(&matches, sub);
    }
    if let Some(("profile-remove", sub)) = matches.subcommand() {
        return run_profile_remove(&matches, sub);
    }
    let routed = route_command(&matches)?;
    let response = send_request(
        &routed.action,
        routed.params,
        routed.profile.as_deref(),
        routed.progress,
    )?;
    let rendered = render_response(&response, routed.json, routed.pretty, routed.full);
    if rendered.is_ok() {
        maybe_runtime_extension_auto_sync(&routed.action, routed.profile.as_deref());
    }
    rendered
}

#[derive(Debug)]
pub(super) struct RoutedCommand {
    pub(super) action: String,
    pub(super) params: Value,
    pub(super) json: bool,
    pub(super) pretty: bool,
    pub(super) full: bool,
    pub(super) progress: bool,
    pub(super) profile: Option<String>,
}
