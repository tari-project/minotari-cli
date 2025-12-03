# Tapplets

Note: When we talk about on-chain data, we are referring to data stored in encrypted data, most usually the memo field.

Tapplets can have tapplet-specific on-chain data are stored as new accounts with view keys. Basically the view key is a hash of the original view key 
and the public key of the tapplet. 
If you know the public key and the view key you can view the tapplet specific data. This filters out data that would be read when using 
the main view key only.

For those wishing to have even more privacy, perhaps in the case where the view key has been provided to another party, tapplet specific data
can be stored in a hash of the private view key, tapplet public key and password.

## Lua

Originally, WASM was used for tapplets. While WASM allows for sandboxing, it does not allow easy support for calling functions with strings and arrays (e.g. public keys as bytes), and would require more plumbing to call simple methods.

Lua is built for small plugins and is a simpler language, making it a good choice.

