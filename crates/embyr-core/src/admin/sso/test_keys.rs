//! Fixed throwaway keypairs for the SSO unit tests (never used outside `cfg(test)`).
//! RSA: same fixture as `oauth_identity`'s tests. EC: P-256, generated once with openssl.

pub(crate) const RSA_PRIVATE_KEY_PEM: &str = "-----BEGIN RSA PRIVATE KEY-----\n\
MIIEogIBAAKCAQEAmAB6fsxR8kVSNO+Oqp9QsYvEmWzgVCPOincm8v9Y0RDRKM4K\n\
n0XshhO6K0XyyGGGbY4huxGzvg82GMXqpijlfe4JpGTfJ9D2SxfvqcWk4lFtf+ba\n\
yJremBNaz0Ji9o5rrAQ9ZuSA7hdkwr9q20uR43xdkErgwRxG2y4FwUjN/d4Te2rI\n\
1YFgCbklsvFmJM8B9LzT4U8vHCWhuqdY3tOCaCw7+U+2L+ktbZfTTpy/QqRNQx70\n\
9sK76Bd0s7eAMj02aO6dBrfZi4lvLntF+6JbVGmzjlckgpxFG+2mmw3m6gwkf371\n\
4yGODJOdTZ4LGuliOR3BALJcRBNDVcbRoYQ6VwIDAQABAoIBADoV1Lmnspj2TJfW\n\
A5rIDroKQzKuHHuKw16+ca/iTDp4RrVlJ0H+IoiJ+VaYAFL6kbhD3Df0Z891WKLW\n\
3vhoIGqjECQ2N+CrRbMkjn09/ehOXZ3Gxkxqgi1zF/yjzdWSTysI473yDCLv5Z1W\n\
MKtkkXdxCwAc3Q5uk9UmHSTjgXRFVfWr8IXivqHsFHJAqE4j706OnHkv/GpoS4eU\n\
lj95+Fl3h/3/LSIRcXCsWyu3Bish8xSRWX1Xgc174yRmcMPxSAQOWBHLdv/ACrSA\n\
9hujwWU4g7BvP+WsgwMhIi2dH02H5xq5XHw6WiLdJNeDJgTo2D4ZIe7jde5MOLRz\n\
i5wHaGECgYEA0fjCWprfriYjv4AF9qH7T/EkkYZP2Zm3zCeGpxkKB3d4KpspkbKe\n\
yuN5pp9EHqIRKRPR0klyi3yz1Rz1fFfofNdVVgzdbvyExeA2RvKKcTdVIIQIcSDg\n\
W8/6ch752+VvNiYuAngthxKlilujsMJ699jH3tjJVnRsGjyOeYNQosMCgYEAuVKN\n\
3y8b0nhOeaNEymrdNR3TRUK0iI3e1DoID6agzGUc482JWIb0ZwP8m2i1ysLBhtJm\n\
OD0GRbZHXQUuJukSE+rqO/nGszmH9/f/yXdO+uiZ+5TOWYqTthvITNNsG0OrpO2S\n\
TErJRmwOS/oyUB9x641rwhl2Z7hTJVR23ISN6N0CgYBHjUrFz3McCFk0P+9ITYiz\n\
hExe3vNFBtIQWwPS24CEbNzhQADZapIcN4pUEoAHJFlOszdUPr0u9W7e18B8AAT0\n\
zfiBm162HI0uVSqJ4Mr2q9FxhCzQSFDMWMJSs2WX3saYIEibhNKW7e7FiFbuvEdl\n\
QFVUBnIN6RyRkENI+0u2OwKBgBk041sh+zTLbFEbJfYqRuA8qEnZYtjYAyD2q7YF\n\
QgXLEvzhLfc+k+uXHTq+KUvk5ZHC+GhZ1IqS2m4KkFZ2iSDwaN+zf5VNE/NkZgQv\n\
GC0Eij0v5klDkgfJC5t3flLPB3+tWKLL4UiU1fT7mPBQ9pvFZozGDdbZuWVwcaJY\n\
3Tx5AoGAcZA04y4sylR+XBtUVLfB2+4H7rb9ysZhPdNgyydmDLZm0O+hFkofQQ1k\n\
DLVyRk7/IK2V0I+g1ygnqz+enhs2H7F2Wi0+zQM11vBMK8oJmoZppgbGM/MZWyDH\n\
QbYkFeEf8V+WdWgKKGU/jVeiJAPwZs9aTEKRDXXd445qPelfou8=\n\
-----END RSA PRIVATE KEY-----\n";

pub(crate) const RSA_N_B64URL: &str = "mAB6fsxR8kVSNO-Oqp9QsYvEmWzgVCPOincm8v9Y0RDRKM4Kn0XshhO6K0XyyGGGbY4huxGzvg82GMXqpijlfe4JpGTfJ9D2SxfvqcWk4lFtf-bayJremBNaz0Ji9o5rrAQ9ZuSA7hdkwr9q20uR43xdkErgwRxG2y4FwUjN_d4Te2rI1YFgCbklsvFmJM8B9LzT4U8vHCWhuqdY3tOCaCw7-U-2L-ktbZfTTpy_QqRNQx709sK76Bd0s7eAMj02aO6dBrfZi4lvLntF-6JbVGmzjlckgpxFG-2mmw3m6gwkf3714yGODJOdTZ4LGuliOR3BALJcRBNDVcbRoYQ6Vw";
pub(crate) const RSA_E_B64URL: &str = "AQAB";

pub(crate) const EC_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgJppL4WPSz6vSxwU0\n\
y/RDbfdBTPfWq34r0l8dHmN0J96hRANCAARoDxr4Ubw2aWFQBE53v4cyaxxh1C/r\n\
D1RWwyDiFLIV+k4/DxsFtvcf9Ahqsd7/lD9+5SKQZM+0zBgrn9dJD3gY\n\
-----END PRIVATE KEY-----\n";
pub(crate) const EC_X_B64URL: &str = "aA8a-FG8NmlhUAROd7-HMmscYdQv6w9UVsMg4hSyFfo";
pub(crate) const EC_Y_B64URL: &str = "Tj8PGwW29x_0CGqx3v-UP37lIpBkz7TMGCuf10kPeBg";
