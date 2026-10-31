#include <jni.h>
#include "client.h"
#include "android_crypto.h"
#include <memory>
#include <mutex>
#include <vector>

namespace {
JavaVM *vm=nullptr;
jclass crypto=nullptr;
jmethodID hmac=nullptr,aes=nullptr;
struct Env {
  JNIEnv *e=nullptr;bool detach=false;
  Env(){if(vm->GetEnv(reinterpret_cast<void**>(&e),JNI_VERSION_1_6)!=JNI_OK){detach=vm->AttachCurrentThread(&e,nullptr)==JNI_OK;}}
  ~Env(){if(detach)vm->DetachCurrentThread();}
};
jbyteArray bytes(JNIEnv *e,const uint8_t *data,size_t n){auto a=e->NewByteArray(static_cast<jsize>(n));if(a && n)e->SetByteArrayRegion(a,0,static_cast<jsize>(n),reinterpret_cast<const jbyte*>(data));return a;}
struct Handle {
  jobject owner=nullptr;SanserMobileClient *client=nullptr;
  jmethodID video=nullptr,audio=nullptr,stopped=nullptr;
  ~Handle(){sanser_mobile_destroy(client);Env e;if(e.e && owner)e.e->DeleteGlobalRef(owner);}
};
// Kotlin serializes handle lifetime. Callbacks only borrow Handle while Peer
// workers are alive; Peer joins those workers before the global ref is freed.
void video(void *context,const uint8_t *data,size_t size,uint32_t width,uint32_t height,int keyframe){
  auto *h=static_cast<Handle*>(context);Env e;if(!e.e || e.e->PushLocalFrame(8)<0)return;
  auto a=bytes(e.e,data,size);if(a)e.e->CallVoidMethod(h->owner,h->video,a,static_cast<jint>(width),static_cast<jint>(height),static_cast<jboolean>(keyframe));
  if(e.e->ExceptionCheck())e.e->ExceptionClear();e.e->PopLocalFrame(nullptr);
}
void audio(void *context,const int16_t *data,size_t size){
  auto *h=static_cast<Handle*>(context);Env e;if(!e.e || e.e->PushLocalFrame(8)<0)return;
  auto a=e.e->NewShortArray(static_cast<jsize>(size));if(a){e.e->SetShortArrayRegion(a,0,static_cast<jsize>(size),data);e.e->CallVoidMethod(h->owner,h->audio,a);}
  if(e.e->ExceptionCheck())e.e->ExceptionClear();e.e->PopLocalFrame(nullptr);
}
void stopped(void *context,unsigned reason){auto *h=static_cast<Handle*>(context);Env e;if(e.e){e.e->CallVoidMethod(h->owner,h->stopped,static_cast<jint>(reason));if(e.e->ExceptionCheck())e.e->ExceptionClear();}}
}
extern "C" JNIEXPORT jint JNICALL JNI_OnLoad(JavaVM *java,void*){
  vm=java;Env e;if(!e.e)return JNI_ERR;
  auto local=e.e->FindClass("com/sanser/mobile/Crypto");if(!local)return JNI_ERR;
  crypto=static_cast<jclass>(e.e->NewGlobalRef(local));e.e->DeleteLocalRef(local);
  hmac=e.e->GetStaticMethodID(crypto,"hmac","([B[B[B)[B");aes=e.e->GetStaticMethodID(crypto,"aes","([B[B[BZ)[B");
  return hmac && aes?JNI_VERSION_1_6:JNI_ERR;
}
extern "C" bool sanser_android_hmac(const uint8_t *key,size_t keyBytes,const uint8_t *a,size_t aBytes,const uint8_t *b,size_t bBytes,uint8_t out[32]){
  Env e;if(!e.e || e.e->PushLocalFrame(8)<0)return false;
  auto result=static_cast<jbyteArray>(e.e->CallStaticObjectMethod(crypto,hmac,bytes(e.e,key,keyBytes),bytes(e.e,a,aBytes),bytes(e.e,b,bBytes)));
  bool ok=!e.e->ExceptionCheck() && result && e.e->GetArrayLength(result)==32;
  if(ok)e.e->GetByteArrayRegion(result,0,32,reinterpret_cast<jbyte*>(out));
  if(e.e->ExceptionCheck())e.e->ExceptionClear();e.e->PopLocalFrame(nullptr);return ok;
}
extern "C" int sanser_android_aes(const uint8_t key[32],const uint8_t iv[16],const uint8_t *input,size_t size,int encrypt,uint8_t *out,size_t capacity){
  Env e;if(!e.e || e.e->PushLocalFrame(8)<0)return -1;
  auto result=static_cast<jbyteArray>(e.e->CallStaticObjectMethod(crypto,aes,bytes(e.e,key,32),bytes(e.e,iv,16),bytes(e.e,input,size),static_cast<jboolean>(encrypt)));
  int n=-1;if(!e.e->ExceptionCheck() && result){auto length=e.e->GetArrayLength(result);if(static_cast<size_t>(length)<=capacity){n=length;e.e->GetByteArrayRegion(result,0,length,reinterpret_cast<jbyte*>(out));}}
  if(e.e->ExceptionCheck())e.e->ExceptionClear();e.e->PopLocalFrame(nullptr);return n;
}
extern "C" JNIEXPORT jlong JNICALL Java_com_sanser_mobile_NativeClient_create(JNIEnv *e,jobject owner,jstring endpoint,jstring token,jboolean control,jboolean sharing,jstring viewer){
  auto h=std::make_unique<Handle>();h->owner=e->NewGlobalRef(owner);auto cls=e->GetObjectClass(owner);
  h->video=e->GetMethodID(cls,"onVideo","([BIIZ)V");h->audio=e->GetMethodID(cls,"onAudio","([S)V");h->stopped=e->GetMethodID(cls,"onStopped","(I)V");
  const char *p=e->GetStringUTFChars(endpoint,nullptr),*t=e->GetStringUTFChars(token,nullptr),*v=e->GetStringUTFChars(viewer,nullptr);
  h->client=sanser_mobile_create(p,0,t,control,sharing,v,{h.get(),video,audio,stopped});
  e->ReleaseStringUTFChars(endpoint,p);e->ReleaseStringUTFChars(token,t);e->ReleaseStringUTFChars(viewer,v);e->DeleteLocalRef(cls);
  if(!h->client || !sanser_mobile_start(h->client))return 0;
  return reinterpret_cast<jlong>(h.release());
}
extern "C" JNIEXPORT void JNICALL Java_com_sanser_mobile_NativeClient_destroy(JNIEnv*,jobject,jlong handle){delete reinterpret_cast<Handle*>(handle);}
extern "C" JNIEXPORT void JNICALL Java_com_sanser_mobile_NativeClient_input(JNIEnv*,jobject,jlong handle,jint kind,jint x,jint y,jint code,jint delta,jboolean down){if(auto *h=reinterpret_cast<Handle*>(handle))sanser_mobile_input(h->client,kind,x,y,code,delta,down);}
extern "C" JNIEXPORT void JNICALL Java_com_sanser_mobile_NativeClient_mute(JNIEnv*,jobject,jlong handle,jboolean value){if(auto *h=reinterpret_cast<Handle*>(handle))sanser_mobile_mute(h->client,value);}
extern "C" JNIEXPORT void JNICALL Java_com_sanser_mobile_NativeClient_keyframe(JNIEnv*,jobject,jlong handle){if(auto *h=reinterpret_cast<Handle*>(handle))sanser_mobile_keyframe(h->client);}
extern "C" JNIEXPORT jdouble JNICALL Java_com_sanser_mobile_NativeClient_rtt(JNIEnv*,jobject,jlong handle){auto *h=reinterpret_cast<Handle*>(handle);return h?sanser_mobile_rtt(h->client):-1;}
