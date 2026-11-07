use buddy_project::Point;

#[derive(Clone)]
pub struct EditSession {
    pub id: String,
    pub origin: Point,
    pub scale: f64,
}
impl EditSession {
    pub fn new(origin: Point, scale: f64) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            origin,
            scale,
        }
    }
    pub fn require(&self, supplied: &str) -> Result<(), String> {
        if self.id == supplied {
            Ok(())
        } else {
            Err("만료됐거나 유효하지 않은 편집 세션입니다.".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_previous_session_cannot_change_a_new_edit() {
        let first = EditSession::new(Point::default(), 1.0);
        let second = EditSession::new(Point::default(), 1.0);
        assert!(second.require(&first.id).is_err());
        assert!(second.require("").is_err());
        assert!(second.require(&second.id).is_ok());
    }
}
