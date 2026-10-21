use super::PluginsManager;
use crate::remote::RemotePluginSummary;
use crate::remote_plugin_id_resolver::persisted_remote_plugin_id_for_installation;
use codex_plugin::PluginId;
use codex_utils_absolute_path::AbsolutePathBuf;
use std::path::Path;

impl PluginsManager {
    /// Fill missing catalog images from the matching installed remote package.
    pub fn populate_remote_plugin_image_paths(&self, summary: &mut RemotePluginSummary) {
        if !summary.installed {
            return;
        }
        let Some(interface) = summary.interface.as_mut() else {
            return;
        };
        let composer_missing =
            interface.composer_icon.is_none() && interface.composer_icon_url.is_none();
        let logo_missing = interface.logo.is_none() && interface.logo_url.is_none();
        let dark_logo_missing = interface.logo_dark.is_none() && interface.logo_url_dark.is_none();
        if !composer_missing && !logo_missing && !dark_logo_missing {
            return;
        }
        let Ok(plugin_id) = PluginId::parse(&summary.id) else {
            return;
        };
        if plugin_id.plugin_name != summary.name {
            return;
        }
        let Some(version) = summary.local_version.as_ref().or(summary.version.as_ref()) else {
            return;
        };
        if self.store.active_plugin_version(&plugin_id).as_ref() != Some(version) {
            return;
        }
        let Some(installation) = self.store.active_plugin_installation(&plugin_id) else {
            return;
        };
        if persisted_remote_plugin_id_for_installation(&installation).as_deref()
            != Some(summary.remote_plugin_id.as_str())
        {
            return;
        }
        let Ok(cache_root) = self.store.root().as_path().canonicalize() else {
            return;
        };
        let Ok(plugin_root) = installation.root.as_path().canonicalize() else {
            return;
        };
        if !plugin_root.starts_with(cache_root) {
            return;
        }
        let Some(loaded) = self.store.manifest_cache.load(installation.root.as_path()) else {
            return;
        };
        let manifest = loaded.manifest;
        if manifest.name != summary.name || manifest.version.as_ref() != Some(version) {
            return;
        }
        let Some(local) = manifest.interface else {
            return;
        };
        if composer_missing {
            interface.composer_icon = contained_image_path(&plugin_root, local.composer_icon);
        }
        if logo_missing {
            interface.logo = contained_image_path(&plugin_root, local.logo);
        }
        if dark_logo_missing {
            interface.logo_dark = contained_image_path(&plugin_root, local.logo_dark);
        }
    }
}

fn contained_image_path(root: &Path, path: Option<AbsolutePathBuf>) -> Option<AbsolutePathBuf> {
    let path = path?.as_path().canonicalize().ok()?;
    if !path.starts_with(root) || !path.is_file() {
        return None;
    }
    AbsolutePathBuf::from_absolute_path_checked(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::REMOTE_CREATED_BY_ME_MARKETPLACE_NAME;
    use crate::test_support::test_plugins_manager;
    use codex_app_server_protocol::PluginAuthPolicy;
    use codex_app_server_protocol::PluginAvailability;
    use codex_app_server_protocol::PluginInstallPolicy;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    fn fixture(
        portable: bool,
    ) -> (
        TempDir,
        PluginsManager,
        RemotePluginSummary,
        AbsolutePathBuf,
    ) {
        let home = TempDir::new().unwrap();
        let manager = test_plugins_manager(home.path().to_path_buf());
        let id = PluginId::new(
            "example".to_string(),
            REMOTE_CREATED_BY_ME_MARKETPLACE_NAME.to_string(),
        )
        .unwrap();
        let root = manager.store.plugin_root(&id, "1.0.0");
        fs::create_dir_all(root.join("assets")).unwrap();
        for name in ["icon.svg", "dark.svg"] {
            fs::write(
                root.join("assets").join(name),
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48"/>"#,
            )
            .unwrap();
        }
        let listing = json!({
            "displayName": "Installed title must not replace catalog metadata",
            "composerIcon": "./assets/icon.svg",
            "logo": "./assets/icon.svg",
            "logoDark": "./assets/dark.svg"
        });
        let (path, manifest) = if portable {
            (
                root.join("plugin.json"),
                json!({
                    "$schema": "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json",
                    "name": "example", "version": "1.0.0",
                    "extensions": {"com.openai": {"interface": listing}}
                }),
            )
        } else {
            fs::create_dir_all(root.join(".codex-plugin")).unwrap();
            (
                root.join(".codex-plugin/plugin.json"),
                json!({"name": "example", "version": "1.0.0", "interface": listing}),
            )
        };
        fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        manager
            .store
            .write_remote_plugin_id(&id, "remote-example")
            .unwrap();
        let summary = RemotePluginSummary {
            id: id.as_key(),
            remote_plugin_id: "remote-example".to_string(),
            version: Some("1.0.0".to_string()),
            local_version: Some("1.0.0".to_string()),
            name: "example".to_string(),
            share_context: None,
            installed: true,
            installed_at: None,
            enabled: false,
            install_policy: PluginInstallPolicy::Available,
            install_policy_source: None,
            must_show_installation_interstitial: None,
            auth_policy: PluginAuthPolicy::OnUse,
            availability: PluginAvailability::Available,
            disabled_reason: None,
            eligible_plan_types: None,
            interface: Some(
                serde_json::from_value(json!({
                    "displayName": "Catalog title", "capabilities": [],
                    "screenshots": [], "screenshotUrls": []
                }))
                .unwrap(),
            ),
            keywords: vec!["catalog".to_string()],
        };
        (home, manager, summary, root)
    }

    fn image(root: &AbsolutePathBuf, name: &str) -> AbsolutePathBuf {
        AbsolutePathBuf::from_absolute_path_checked(
            root.join("assets").join(name).canonicalize().unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn fills_images_from_portable_and_legacy_packages_without_changing_other_metadata() {
        for portable in [true, false] {
            let (_home, manager, mut summary, root) = fixture(portable);
            let mut expected = summary.clone();
            let interface = expected.interface.as_mut().unwrap();
            interface.composer_icon = Some(image(&root, "icon.svg"));
            interface.logo = Some(image(&root, "icon.svg"));
            interface.logo_dark = Some(image(&root, "dark.svg"));
            manager.populate_remote_plugin_image_paths(&mut summary);
            assert_eq!(summary, expected);
        }
    }

    #[test]
    fn preserves_remote_urls_and_only_fills_missing_images() {
        let (_home, manager, mut summary, root) = fixture(true);
        let interface = summary.interface.as_mut().unwrap();
        interface.composer_icon_url = Some("https://example.com/composer.svg".to_string());
        interface.logo_url_dark = Some("https://example.com/dark.svg".to_string());
        let mut expected = summary.clone();
        expected.interface.as_mut().unwrap().logo = Some(image(&root, "icon.svg"));
        manager.populate_remote_plugin_image_paths(&mut summary);
        assert_eq!(summary, expected);
    }

    #[test]
    fn installed_endpoint_can_use_its_version_when_local_version_is_absent() {
        let (_home, manager, mut summary, root) = fixture(true);
        summary.local_version = None;
        manager.populate_remote_plugin_image_paths(&mut summary);
        assert_eq!(
            summary.interface.unwrap().logo,
            Some(image(&root, "icon.svg"))
        );
    }

    #[test]
    fn refuses_uninstalled_stale_or_mismatched_identity() {
        for mismatch in ["uninstalled", "version", "remote-id", "name", "id"] {
            let (_home, manager, mut summary, _root) = fixture(true);
            match mismatch {
                "uninstalled" => summary.installed = false,
                "version" => summary.local_version = Some("2.0.0".to_string()),
                "remote-id" => summary.remote_plugin_id = "different-remote".to_string(),
                "name" => summary.name = "different-name".to_string(),
                "id" => summary.id = "../example@created-by-me-remote".to_string(),
                _ => unreachable!(),
            }
            let before = summary.clone();
            manager.populate_remote_plugin_image_paths(&mut summary);
            assert_eq!(summary, before, "{mismatch}");
        }
    }

    #[test]
    fn rejects_missing_assets_directories_and_traversal_paths() {
        for path in ["./missing.svg", "./assets", "./../outside.svg"] {
            let (_home, manager, mut summary, root) = fixture(false);
            fs::write(
                root.join(".codex-plugin/plugin.json"),
                serde_json::to_vec(&json!({
                    "name": "example", "version": "1.0.0",
                    "interface": {"logo": path}
                }))
                .unwrap(),
            )
            .unwrap();
            let before = summary.clone();
            manager.populate_remote_plugin_image_paths(&mut summary);
            assert_eq!(summary, before, "{path}");
        }
    }

    #[test]
    fn refuses_a_manifest_with_a_different_identity_or_version() {
        for field in ["name", "version"] {
            let (_home, manager, mut summary, root) = fixture(true);
            let path = root.join("plugin.json");
            let mut manifest: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            manifest[field] = json!("different");
            fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
            let before = summary.clone();
            manager.populate_remote_plugin_image_paths(&mut summary);
            assert_eq!(summary, before, "{field}");
        }
    }

    #[test]
    fn refuses_a_cache_without_the_persisted_remote_identity() {
        let (_home, manager, mut summary, _root) = fixture(true);
        let id = PluginId::parse(&summary.id).unwrap();
        fs::remove_file(
            manager
                .store
                .plugin_base_root(&id)
                .join(".codex-remote-plugin-install.json"),
        )
        .unwrap();
        let before = summary.clone();
        manager.populate_remote_plugin_image_paths(&mut summary);
        assert_eq!(summary, before);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_an_asset_symlink_outside_the_installed_package() {
        let (home, manager, mut summary, root) = fixture(false);
        let outside = home.path().join("outside.svg");
        fs::write(&outside, "outside").unwrap();
        fs::remove_file(root.join("assets/icon.svg")).unwrap();
        std::os::unix::fs::symlink(outside, root.join("assets/icon.svg")).unwrap();
        manager.populate_remote_plugin_image_paths(&mut summary);
        let interface = summary.interface.unwrap();
        assert_eq!(interface.composer_icon, None);
        assert_eq!(interface.logo, None);
        assert_eq!(interface.logo_dark, Some(image(&root, "dark.svg")));
    }
}
