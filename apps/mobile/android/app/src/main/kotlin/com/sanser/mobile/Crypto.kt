package com.sanser.mobile

import javax.crypto.Cipher
import javax.crypto.Mac
import javax.crypto.spec.IvParameterSpec
import javax.crypto.spec.SecretKeySpec

/** Platform primitives for the existing authenticated SNV2 wire format. */
object Crypto {
    @JvmStatic fun hmac(key: ByteArray, first: ByteArray, second: ByteArray): ByteArray =
        Mac.getInstance("HmacSHA256").run {
            init(SecretKeySpec(key, "HmacSHA256")); update(first); doFinal(second)
        }
    @JvmStatic fun aes(key: ByteArray, iv: ByteArray, input: ByteArray, encrypt: Boolean): ByteArray =
        Cipher.getInstance("AES/CBC/PKCS5Padding").run {
            init(if (encrypt) Cipher.ENCRYPT_MODE else Cipher.DECRYPT_MODE, SecretKeySpec(key, "AES"), IvParameterSpec(iv))
            doFinal(input)
        }
}
