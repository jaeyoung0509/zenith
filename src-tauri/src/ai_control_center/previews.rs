use crate::models::{Recommendation, RecommendationPreview};
use std::collections::HashMap;
use uuid::Uuid;
const TTL: u64 = 120;
#[derive(Debug, Default)]
pub struct PreviewStore {
    items: HashMap<String, RecommendationPreview>,
}
impl PreviewStore {
    pub fn create(&mut self, recommendation: &Recommendation, now: u64) -> RecommendationPreview {
        self.items.retain(|_, item| item.expires_at > now);
        if self.items.len() >= 64 {
            if let Some(id) = self
                .items
                .values()
                .min_by_key(|item| item.expires_at)
                .map(|item| item.id.clone())
            {
                self.items.remove(&id);
            }
        }
        let preview = RecommendationPreview {
            id: Uuid::new_v4().to_string(),
            recommendation_id: recommendation.id.clone(),
            title: recommendation.title.clone(),
            explanation: recommendation.message.clone(),
            destination: recommendation.destination,
            action_label: recommendation
                .action_label
                .clone()
                .unwrap_or_else(|| "Review".into()),
            expires_at: now + TTL,
        };
        self.items.insert(preview.id.clone(), preview.clone());
        preview
    }
    pub fn consume(&mut self, id: &str, now: u64) -> Result<RecommendationPreview, String> {
        let item = self
            .items
            .remove(id)
            .ok_or_else(|| "Recommendation preview not found or already used".to_string())?;
        if item.expires_at <= now {
            return Err("Recommendation preview expired".into());
        }
        Ok(item)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::RecommendationKind;
    fn recommendation() -> Recommendation {
        Recommendation {
            id: "r".into(),
            kind: RecommendationKind::Memory,
            title: "Review".into(),
            message: "Advisory only".into(),
            created_at: 1,
            cooldown_until: 2,
            session_id: None,
            project_id: None,
            action_label: Some("Open Memory".into()),
            destination: crate::models::DashboardRoute::Memory,
        }
    }
    #[test]
    fn preview_is_opaque_expiring_and_one_shot() {
        let mut store = PreviewStore::default();
        let item = store.create(&recommendation(), 10);
        assert_eq!(item.destination, crate::models::DashboardRoute::Memory);
        assert_eq!(item.action_label, "Open Memory");
        assert!(store.consume(&item.id, 11).is_ok());
        assert!(store.consume(&item.id, 11).is_err());
        let expired = store.create(&recommendation(), 20);
        assert!(store.consume(&expired.id, 140).is_err());
    }
}
