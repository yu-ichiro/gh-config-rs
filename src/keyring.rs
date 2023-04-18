#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[cfg(target_os = "windows")]
    #[error(transparent)]
    Windows(#[from] windows::Error),

    #[cfg(target_os = "macos")]
    #[error(transparent)]
    Macos(#[from] macos::Error),

    #[cfg(target_os = "linux")]
    #[error(transparent)]
    Macos(#[from] linux::Error),
}

type Result<T> = std::result::Result<T, Error>;

pub trait GhKeyring {
    fn get(&self, host: &str) -> Result<Option<Vec<u8>>>;
}

#[cfg(target_os = "windows")]
mod windows {
    use super::*;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{GetLastError, ERROR_NOT_FOUND, WIN32_ERROR};
    use windows::Win32::Security::Credentials::{
        CredFree, CredReadW, CREDENTIALW, CRED_TYPE_GENERIC,
    };

    #[derive(Debug, thiserror::Error)]
    pub enum Error {
        #[error("Win32 API error: {0}")]
        Win32(WIN32_ERROR),
    }

    pub struct Wincred;

    impl GhKeyring for Wincred {
        fn get(&self, host: &str) -> super::Result<Option<Vec<u8>>> {
            let credential = MaybeUninit::<*mut CREDENTIALW>::uninit();
            let name = PCWSTR::from_raw(
                format!("gh:{}:")
                    .encode_utf16()
                    .chain(once(0))
                    .collect::<Vec<_>>()
                    .as_ptr(),
            );

            match unsafe { CredReadW(name, CRED_TYPE_GENERIC, 0, credential.as_mut_ptr()) } {
                0 => match unsafe { GetLastError() } {
                    ERROR_NOT_FOUND => Ok(None),
                    e => Err(Error::Win32(e)),
                },
                _ => {
                    let credential = unsafe { credential.assume_init() };
                    let token = unsafe {
                        std::slice::from_raw_parts(
                            (&*credential).CredentialBlob,
                            (&*credential).CredentialBlobSize,
                        )
                    }
                    .to_vec();

                    unsafe { CredFree(credential as *mut _) };

                    Ok(Some(token))
                }
            }
            .map_err(super::Error::Windows)
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use base64::Engine;
    use security_framework::os::macos::keychain::{SecKeychain, SecPreferencesDomain};

    #[derive(Debug, thiserror::Error)]
    pub enum Error {
        #[error("Security.framework returned an error: {0}")]
        SecurityFramework(#[from] security_framework::base::Error),

        #[error("The encoded token is invalid as a hex string: {0}")]
        InvalidHex(#[from] hex::FromHexError),

        #[error("The encoded token is invalid as a Base64 string: {0}")]
        InvalidBase64(#[from] base64::DecodeError),

        #[error("The token is formatted in an unknown encoding.")]
        UnknownFormat,
    }

    type Result<T> = std::result::Result<T, Error>;

    enum GhEncodedToken {
        Hex(Vec<u8>),
        Base64(Vec<u8>),
    }

    impl TryFrom<&[u8]> for GhEncodedToken {
        type Error = Error;

        fn try_from(value: &[u8]) -> Result<Self> {
            if let Some(s) = value.strip_prefix(b"go-keyring-encoded:") {
                return Ok(Self::Hex(s.to_vec()));
            }

            if let Some(s) = value.strip_prefix(b"go-keyring-base64:") {
                return Ok(Self::Base64(s.to_vec()));
            }

            Err(Error::UnknownFormat)
        }
    }

    impl GhEncodedToken {
        fn decode(&self) -> Result<Vec<u8>> {
            Ok(match self {
                Self::Hex(s) => hex::decode(s)?,
                Self::Base64(s) => base64::engine::general_purpose::STANDARD.decode(s)?,
            })
        }
    }

    pub struct Keychain;

    impl GhKeyring for Keychain {
        fn get(&self, host: &str) -> super::Result<Option<Vec<u8>>> {
            (|| match SecKeychain::default_for_domain(SecPreferencesDomain::User)?
                .find_generic_password(format!("gh:{}", host).as_str(), "")
            {
                Ok((token, _)) => GhEncodedToken::try_from(token.as_ref())
                    .and_then(|t| t.decode())
                    .map(Some),
                Err(e) => match e.code() {
                    -25300 => Ok(None),
                    _ => Err(e.into()),
                },
            })()
            .map_err(super::Error::Macos)
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    #[derive(Debug, thiserror::Error)]
    pub enum Error {}
}

#[cfg(target_os = "windows")]
pub use windows::Wincred as Keyring;

#[cfg(target_os = "macos")]
pub use macos::Keychain as Keyring;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_from_keyring() {
        assert!(Keyring.get("github.com").unwrap().is_some());
    }
}
