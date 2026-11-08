use super::*;

impl App {
    pub(super) fn persist_session_updates(&mut self) -> Result<()> {
        if self.demo {
            self.persisted_session = None;
            return Ok(());
        }
        persist_current_session(
            &self.session_file,
            &mut self.persisted_session,
            self.jobs.session(),
        )?;
        Ok(())
    }
}

fn persist_current_session(
    file: &SessionFile,
    persisted: &mut Option<source::AuthSession>,
    current: Option<source::AuthSession>,
) -> Result<bool> {
    let Some(current) = current else {
        // Logout owns file removal; entering demo must preserve the real account.
        *persisted = None;
        return Ok(false);
    };
    if persisted.as_ref() == Some(&current) {
        return Ok(false);
    }
    file.save(&current)?;
    *persisted = Some(current);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{AuthSession, WeReadClient};

    fn account(identity: &str, cookie: &str) -> AuthSession {
        AuthSession {
            api_key: format!("fixture-key-{identity}"),
            user_vid: identity.into(),
            cookies: BTreeMap::from([("wr_skey".into(), cookie.into())]),
            ..Default::default()
        }
    }

    #[test]
    fn shared_client_updates_are_persisted_without_rewriting_unchanged_material() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let file = SessionFile::new(directory.path());
        let initial = account("fixture-account-a", "fixture-initial-cookie");
        file.save(&initial)?;
        let mut persisted = Some(initial.clone());
        let client = WeReadClient::new(initial)?;
        let runtime = tokio::runtime::Runtime::new()?;
        let (sender, _) = mpsc::channel();
        let jobs = Jobs::new(runtime.handle().clone(), sender, Some(client.clone()))?;
        assert!(!persist_current_session(
            &file,
            &mut persisted,
            jobs.session()
        )?);

        let mut updated = client.session();
        updated
            .cookies
            .insert("wr_skey".into(), "fixture-rotated-cookie".into());
        updated.wr_ticket = "fixture-updated-ticket".into();
        client.replace_session(updated.clone())?;

        assert!(persist_current_session(
            &file,
            &mut persisted,
            jobs.session()
        )?);
        assert_eq!(file.load()?, Some(updated.clone()));
        assert_eq!(persisted, Some(updated));
        assert!(!persist_current_session(
            &file,
            &mut persisted,
            jobs.session()
        )?);
        Ok(())
    }

    #[test]
    fn an_old_client_cannot_restore_logged_out_or_replaced_account_material() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let file = SessionFile::new(directory.path());
        let initial = account("fixture-account-a", "fixture-initial-cookie");
        file.save(&initial)?;
        let mut persisted = Some(initial.clone());
        let old_client = WeReadClient::new(initial)?;
        let runtime = tokio::runtime::Runtime::new()?;
        let (sender, _) = mpsc::channel();
        let jobs = Jobs::new(runtime.handle().clone(), sender, Some(old_client.clone()))?;

        jobs.clear_session();
        file.clear()?;
        old_client.replace_session(account("fixture-account-a", "fixture-late-cookie"))?;
        assert!(!persist_current_session(
            &file,
            &mut persisted,
            jobs.session()
        )?);
        assert!(persisted.is_none());
        assert!(file.load()?.is_none());

        let replacement = account("fixture-account-b", "fixture-account-b-cookie");
        jobs.replace_session(replacement.clone())?;
        assert!(persist_current_session(
            &file,
            &mut persisted,
            jobs.session()
        )?);
        old_client.replace_session(account("fixture-account-a", "fixture-even-later-cookie"))?;
        assert!(!persist_current_session(
            &file,
            &mut persisted,
            jobs.session()
        )?);
        assert_eq!(file.load()?, Some(replacement));
        Ok(())
    }

    #[test]
    fn a_failed_save_keeps_the_previous_snapshot_for_a_later_retry() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let file = SessionFile::new(directory.path());
        let initial = account("fixture-account-a", "fixture-initial-cookie");
        file.save(&initial)?;
        let mut persisted = Some(initial.clone());
        let updated = account("fixture-account-a", "fixture-rotated-cookie");
        let blocker = directory.path().join("account.pending");
        std::fs::create_dir(&blocker)?;

        assert!(persist_current_session(&file, &mut persisted, Some(updated.clone())).is_err());
        assert_eq!(persisted, Some(initial.clone()));
        assert_eq!(file.load()?, Some(initial));
        std::fs::remove_dir(blocker)?;
        assert!(persist_current_session(
            &file,
            &mut persisted,
            Some(updated.clone())
        )?);
        assert_eq!(file.load()?, Some(updated));
        Ok(())
    }
}
