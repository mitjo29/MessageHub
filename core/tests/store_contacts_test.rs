use chrono::Utc;
use messagehub_core::store::Store;
use messagehub_core::types::*;
use uuid::Uuid;

fn test_store() -> Store {
    Store::open_in_memory().unwrap()
}

#[test]
fn test_insert_and_get_contact() {
    let store = test_store();
    let contact = Contact {
        id: Uuid::new_v4(),
        display_name: "Sarah Chen".into(),
        identities: vec![
            ContactIdentity { channel: Channel::Email, address: "sarah@example.com".into() },
            ContactIdentity { channel: Channel::Telegram, address: "@sarachen".into() },
        ],
        vault_ref: Some("05-People/Sarah Chen.md".into()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    store.insert_contact(&contact).unwrap();

    let retrieved = store.get_contact(&contact.id).unwrap();
    assert_eq!(retrieved.display_name, "Sarah Chen");
    assert_eq!(retrieved.identities.len(), 2);
    assert_eq!(retrieved.vault_ref.as_deref(), Some("05-People/Sarah Chen.md"));
}

#[test]
fn test_find_contact_by_address() {
    let store = test_store();
    let contact = Contact {
        id: Uuid::new_v4(),
        display_name: "Sarah Chen".into(),
        identities: vec![
            ContactIdentity { channel: Channel::Email, address: "sarah@example.com".into() },
        ],
        vault_ref: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    store.insert_contact(&contact).unwrap();

    let found = store.find_contact_by_address(Channel::Email, "sarah@example.com").unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().id, contact.id);

    let not_found = store.find_contact_by_address(Channel::Email, "nobody@example.com").unwrap();
    assert!(not_found.is_none());
}

#[test]
fn test_merge_contact_identities() {
    let store = test_store();
    let contact = Contact {
        id: Uuid::new_v4(),
        display_name: "Sarah Chen".into(),
        identities: vec![
            ContactIdentity { channel: Channel::Email, address: "sarah@example.com".into() },
        ],
        vault_ref: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    store.insert_contact(&contact).unwrap();

    let new_identity = ContactIdentity { channel: Channel::WhatsApp, address: "+491234567".into() };
    store.add_identity(&contact.id, &new_identity).unwrap();

    let retrieved = store.get_contact(&contact.id).unwrap();
    assert_eq!(retrieved.identities.len(), 2);
}
