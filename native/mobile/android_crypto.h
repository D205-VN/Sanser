#pragma once
#include <stddef.h>
#include <stdint.h>
// Android delegates cryptographic primitives to JCA. Wire/authentication and
// replay rules remain the same shared SNV2 implementation as desktop.
extern "C" bool sanser_android_hmac(const uint8_t *key,size_t keyBytes,const uint8_t *a,size_t aBytes,const uint8_t *b,size_t bBytes,uint8_t *output);
extern "C" int sanser_android_aes(const uint8_t *key,const uint8_t *iv,const uint8_t *input,size_t size,int encrypt,uint8_t *output,size_t capacity);
