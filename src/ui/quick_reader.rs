//! Clipboard/selection reads used by Quick Assist.
use anyhow::Result;

pub(super) async fn read(
    reader: impl Fn() -> Result<String>,
    retry: bool,
    current: impl Fn() -> bool,
) -> Result<Option<String>> {
    if !current() {
        return Ok(None);
    }
    if retry {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
    let mut text = Err(anyhow::anyhow!("Zwischenablage nicht verfügbar"));
    for _ in 0..if retry { 4 } else { 1 } {
        if !current() {
            return Ok(None);
        }
        text = reader();
        if !current() {
            return Ok(None);
        }
        if text.is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    if !current() {
        return Ok(None);
    }
    text.map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[tokio::test]
    async fn replaced_request_discards_late_selection() {
        let revision = Cell::new(1);
        let result = read(
            || {
                revision.set(2);
                Ok("old selection".into())
            },
            false,
            || revision.get() == 1,
        )
        .await
        .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn replaced_request_does_not_retry_or_publish_an_old_error() {
        let revision = Cell::new(1);
        let attempts = Cell::new(0);
        let result = read(
            || {
                attempts.set(attempts.get() + 1);
                revision.set(2);
                anyhow::bail!("old clipboard failure")
            },
            true,
            || revision.get() == 1,
        )
        .await;
        assert!(result.unwrap().is_none());
        assert_eq!(attempts.get(), 1);
    }

    #[tokio::test]
    async fn newest_request_keeps_its_text_and_current_errors() {
        assert_eq!(
            read(|| Ok("new selection".into()), false, || true)
                .await
                .unwrap()
                .as_deref(),
            Some("new selection")
        );
        assert!(
            read(|| anyhow::bail!("current failure"), false, || true)
                .await
                .is_err()
        );
    }
}
