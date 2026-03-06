use super::*;

pub(super) fn build_cli() -> Command {
    Command::new("tabctl")
        .disable_help_subcommand(true)
        .arg(
            Arg::new("json")
                .long("json")
                .action(ArgAction::SetTrue)
                .global(true),
        )
        .arg(
            Arg::new("pretty")
                .long("pretty")
                .action(ArgAction::SetTrue)
                .global(true),
        )
        .arg(
            Arg::new("no-pretty")
                .long("no-pretty")
                .action(ArgAction::SetTrue)
                .global(true),
        )
        .arg(
            Arg::new("profile")
                .long("profile")
                .value_name("name")
                .global(true),
        )
        .arg(
            Arg::new("progress")
                .long("progress")
                .action(ArgAction::SetTrue)
                .global(true),
        )
        .arg(
            Arg::new("version")
                .long("version")
                .short('v')
                .action(ArgAction::SetTrue)
                .global(true),
        )
        .subcommand(command_ping())
        .subcommand(command_history())
        .subcommand(command_setup())
        .subcommand(command_upgrade())
        .subcommand(command_doctor())
        .subcommand(command_policy())
        .subcommand(command_profile_list())
        .subcommand(command_profile_show())
        .subcommand(command_profile_switch())
        .subcommand(command_profile_remove())
        .subcommand(command_help())
        .subcommand(command_version())
        .subcommand(command_extension_fetch())
        .subcommand(command_query())
        .subcommand(Command::new("schema"))
}

pub(super) fn command_ping() -> Command {
    Command::new("ping")
}

pub(super) fn command_history() -> Command {
    Command::new("history").arg(
        Arg::new("limit")
            .long("limit")
            .value_parser(value_parser!(u64))
            .value_name("n"),
    )
}

pub(super) fn command_query() -> Command {
    Command::new("query").arg(
        Arg::new("graphql")
            .required(true)
            .index(1)
            .value_name("QUERY"),
    )
}

pub(super) fn command_help() -> Command {
    Command::new("help").arg(Arg::new("command").value_name("command").index(1))
}

pub(super) fn command_version() -> Command {
    Command::new("version")
}

pub(super) fn command_upgrade() -> Command {
    Command::new("upgrade").visible_alias("update")
}

pub(super) fn command_doctor() -> Command {
    Command::new("doctor").arg(Arg::new("fix").long("fix").action(ArgAction::SetTrue))
}

pub(super) fn command_policy() -> Command {
    Command::new("policy").arg(Arg::new("init").long("init").action(ArgAction::SetTrue))
}

pub(super) fn command_profile_list() -> Command {
    Command::new("profile-list")
}

pub(super) fn command_profile_show() -> Command {
    Command::new("profile-show")
}

pub(super) fn command_profile_switch() -> Command {
    Command::new("profile-switch").arg(Arg::new("name").value_name("name").required(true))
}

pub(super) fn command_profile_remove() -> Command {
    Command::new("profile-remove").arg(Arg::new("name").value_name("name").required(true))
}

pub(super) fn command_setup() -> Command {
    Command::new("setup")
        .arg(
            Arg::new("browser")
                .long("browser")
                .required(true)
                .value_name("edge|chrome"),
        )
        .arg(
            Arg::new("skip-extension-download")
                .long("skip-extension-download")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("release-repo")
                .long("release-repo")
                .value_name("owner/repo"),
        )
        .arg(
            Arg::new("release-tag")
                .long("release-tag")
                .value_name("tag|version"),
        )
        .arg(
            Arg::new("release-version")
                .long("release-version")
                .value_name("version"),
        )
        .arg(
            Arg::new("release-asset")
                .long("release-asset")
                .value_name("name"),
        )
        .arg(
            Arg::new("extension-id")
                .long("extension-id")
                .value_name("id"),
        )
        .arg(
            Arg::new("extension-dir")
                .long("extension-dir")
                .value_name("path"),
        )
        .arg(Arg::new("node").long("node").value_name("path"))
        .arg(Arg::new("name").long("name").value_name("name"))
        .arg(
            Arg::new("user-data-dir")
                .long("user-data-dir")
                .value_name("path"),
        )
}

pub(super) fn command_extension_fetch() -> Command {
    Command::new("extension-fetch")
        .arg(
            Arg::new("version")
                .long("version")
                .value_name("version|tag"),
        )
        .arg(
            Arg::new("repo")
                .long("repo")
                .value_name("owner/repo")
                .default_value("ekroon/tabctl"),
        )
        .arg(
            Arg::new("asset")
                .long("asset")
                .value_name("name")
                .default_value("tabctl-extension.zip"),
        )
        .arg(Arg::new("out").long("out").value_name("path"))
}
