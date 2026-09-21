#![cfg(any(target_os = "macos", target_os = "windows"))]

use keyring::Entry;

#[test]
fn native_credential_backend_is_selected() -> Result<(), Box<dyn std::error::Error>> {
    let entry = Entry::new("com.sanser.desktop.persistence-test", "backend-selection")?;
    #[cfg(target_os = "macos")]
    assert!(entry.get_credential().is::<keyring::macos::MacCredential>());
    #[cfg(target_os = "windows")]
    assert!(
        entry
            .get_credential()
            .is::<keyring::windows::WinCredential>()
    );
    Ok(())
}

#[test]
#[ignore = "writes and deletes an isolated synthetic credential in the OS credential store"]
fn credential_survives_process_restart() -> Result<(), Box<dyn std::error::Error>> {
    const SERVICE: &str = "com.sanser.desktop.persistence-test";
    const VALUE: &str = "sanser-synthetic-persistence-probe";
    if let Ok(identifier) = std::env::var("SANSER_PERSISTENCE_TEST_ID") {
        let identifier = uuid::Uuid::parse_str(&identifier)?.to_string();
        let entry = Entry::new(SERVICE, &identifier)?;
        if std::env::var("SANSER_PERSISTENCE_TEST_PHASE")? == "write" {
            entry.set_password(VALUE)?;
        } else {
            assert_eq!(entry.get_password()?, VALUE);
        }
        return Ok(());
    }
    let identifier = uuid::Uuid::new_v4().to_string();
    let entry = Entry::new(SERVICE, &identifier)?;
    let outcome = (|| -> Result<(), Box<dyn std::error::Error>> {
        for phase in ["write", "read"] {
            let result = std::process::Command::new(std::env::current_exe()?)
                .args([
                    "--ignored",
                    "--exact",
                    "credential_survives_process_restart",
                ])
                .env("SANSER_PERSISTENCE_TEST_ID", &identifier)
                .env("SANSER_PERSISTENCE_TEST_PHASE", phase)
                .output()?;
            if !result.status.success() {
                return Err(format!("Synthetic credential {phase} process failed").into());
            }
        }
        Ok(())
    })();
    let cleanup = entry.delete_credential();
    outcome?;
    cleanup?;
    Ok(())
}
