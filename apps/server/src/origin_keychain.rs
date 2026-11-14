use super::*;
use core_foundation::{
    base::{CFType, TCFType},
    boolean::CFBoolean,
    data::CFData,
    dictionary::CFDictionary,
    string::{CFString, CFStringRef},
};
use security_framework_sys::{
    item::*,
    keychain_item::{SecItemAdd, SecItemCopyMatching, SecItemDelete},
};

// The sys crate exposes the UI key and Skip. Fail returns a structured error
// instead of silently omitting a locked match or presenting a user prompt.
unsafe extern "C" {
    static kSecUseAuthenticationUIFail: CFStringRef;
    static kSecMatchLimitOne: CFStringRef;
}
const SERVICE: &str = "com.nexum.server.origin-credentials.v1";
fn status(value: i32) -> Result<(), ProviderError> {
    match value {
        0 => Ok(()),
        -25300 => Err(ProviderError::Missing),
        -25308 => Err(ProviderError::Locked),
        -25293 | -128 => Err(ProviderError::Denied),
        -25299 => Err(ProviderError::Exists),
        _ => Err(ProviderError::Unavailable),
    }
}
fn query(reference: &CredentialRef) -> Vec<(CFString, CFType)> {
    // Structured array encoding prevents ambiguous account components.
    let account = json!([
        reference.instance_id,
        reference.origin,
        reference.credential_id,
        reference.revision
    ])
    .to_string();
    unsafe {
        vec![
            (
                CFString::wrap_under_get_rule(kSecClass),
                CFString::wrap_under_get_rule(kSecClassGenericPassword).into_CFType(),
            ),
            (
                CFString::wrap_under_get_rule(kSecAttrService),
                CFString::new(SERVICE).into_CFType(),
            ),
            (
                CFString::wrap_under_get_rule(kSecAttrAccount),
                CFString::new(&account).into_CFType(),
            ),
            (
                CFString::wrap_under_get_rule(kSecAttrSynchronizable),
                CFBoolean::false_value().into_CFType(),
            ),
            (
                CFString::wrap_under_get_rule(kSecUseAuthenticationUI),
                CFString::wrap_under_get_rule(kSecUseAuthenticationUIFail).into_CFType(),
            ),
        ]
    }
}
impl CredentialProvider for SystemProvider {
    fn durable(&self) -> bool {
        true
    }
    fn put(&self, reference: &CredentialRef, secret: &OriginSecret) -> Result<(), ProviderError> {
        let data = serde_json::to_vec(&[&secret.username, &secret.password])
            .map_err(|_| ProviderError::Unavailable)?;
        let mut fields = query(reference);
        fields.push((
            unsafe { CFString::wrap_under_get_rule(kSecValueData) },
            CFData::from_buffer(&data).into_CFType(),
        ));
        let dictionary = CFDictionary::from_CFType_pairs(&fields);
        status(unsafe { SecItemAdd(dictionary.as_concrete_TypeRef(), std::ptr::null_mut()) })
    }
    fn get(&self, reference: &CredentialRef) -> Result<OriginSecret, ProviderError> {
        let mut fields = query(reference);
        unsafe {
            fields.push((
                CFString::wrap_under_get_rule(kSecReturnData),
                CFBoolean::true_value().into_CFType(),
            ));
            fields.push((
                CFString::wrap_under_get_rule(kSecMatchLimit),
                CFString::wrap_under_get_rule(kSecMatchLimitOne).into_CFType(),
            ));
        }
        let dictionary = CFDictionary::from_CFType_pairs(&fields);
        let mut result = std::ptr::null();
        status(unsafe { SecItemCopyMatching(dictionary.as_concrete_TypeRef(), &mut result) })?;
        if result.is_null() {
            return Err(ProviderError::Unavailable);
        }
        let result = unsafe { CFType::wrap_under_create_rule(result) };
        let data = result
            .downcast::<CFData>()
            .ok_or(ProviderError::Unavailable)?;
        let [username, password]: [String; 2] =
            serde_json::from_slice(data.bytes()).map_err(|_| ProviderError::Unavailable)?;
        Ok(OriginSecret { username, password })
    }
    fn remove(&self, reference: &CredentialRef) -> Result<(), ProviderError> {
        let dictionary = CFDictionary::from_CFType_pairs(&query(reference));
        match status(unsafe { SecItemDelete(dictionary.as_concrete_TypeRef()) }) {
            Err(ProviderError::Missing) => Ok(()),
            result => result,
        }
    }
}
