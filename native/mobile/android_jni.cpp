#include <jni.h>
#include "client.h"
#include "p2p.h"
#include "android_crypto.h"
#include <memory>
#include <mutex>
#include <vector>
#include <algorithm>
#include <cstring>

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
  jmethodID video=nullptr,audio=nullptr,stopped=nullptr,clipboard=nullptr;
  ~Handle(){sanser_mobile_destroy(client);Env e;if(e.e && owner)e.e->DeleteGlobalRef(owner);}
};
struct ClipboardCall {
  jobject owner=nullptr;jmethodID method=nullptr;jlong request=0;
  ~ClipboardCall(){Env e;if(e.e && owner)e.e->DeleteGlobalRef(owner);}
};
void clipboardReply(void* context,unsigned status,const uint8_t* text,size_t count) {
  std::unique_ptr<ClipboardCall> call(static_cast<ClipboardCall*>(context));Env e;
  if(!e.e || e.e->PushLocalFrame(4)<0)return;
  const auto data=bytes(e.e,text,count);
  if(data)e.e->CallVoidMethod(call->owner,call->method,call->request,static_cast<jint>(status),data);
  if(e.e->ExceptionCheck())e.e->ExceptionClear();e.e->PopLocalFrame(nullptr);
}
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
extern "C" JNIEXPORT jlong JNICALL Java_com_sanser_mobile_NativeClient_create(JNIEnv *e,jobject owner,jstring endpoint,jint port,jstring token,jboolean control,jboolean sharing,jstring viewer,jboolean files){
  auto h=std::make_unique<Handle>();h->owner=e->NewGlobalRef(owner);auto cls=e->GetObjectClass(owner);
  h->video=e->GetMethodID(cls,"onVideo","([BIIZ)V");h->audio=e->GetMethodID(cls,"onAudio","([S)V");h->stopped=e->GetMethodID(cls,"onStopped","(I)V");
  h->clipboard=e->GetMethodID(cls,"onClipboard","(JI[B)V");
  const char *p=e->GetStringUTFChars(endpoint,nullptr),*t=e->GetStringUTFChars(token,nullptr),*v=e->GetStringUTFChars(viewer,nullptr);
  h->client=sanser_mobile_create(p,static_cast<uint16_t>(port),t,control,sharing,v,{h.get(),video,audio,stopped});
  e->ReleaseStringUTFChars(endpoint,p);e->ReleaseStringUTFChars(token,t);e->ReleaseStringUTFChars(viewer,v);e->DeleteLocalRef(cls);
  if(!h->client)return 0;sanser_mobile_files_enable(h->client,files);if(!sanser_mobile_start(h->client))return 0;
  return reinterpret_cast<jlong>(h.release());
}
extern "C" JNIEXPORT void JNICALL Java_com_sanser_mobile_NativeClient_destroy(JNIEnv*,jobject,jlong handle){delete reinterpret_cast<Handle*>(handle);}
extern "C" JNIEXPORT void JNICALL Java_com_sanser_mobile_NativeClient_input(JNIEnv*,jobject,jlong handle,jint kind,jint x,jint y,jint code,jint delta,jboolean down){if(auto *h=reinterpret_cast<Handle*>(handle))sanser_mobile_input(h->client,kind,x,y,code,delta,down);}
extern "C" JNIEXPORT void JNICALL Java_com_sanser_mobile_NativeClient_mute(JNIEnv*,jobject,jlong handle,jboolean value){if(auto *h=reinterpret_cast<Handle*>(handle))sanser_mobile_mute(h->client,value);}
extern "C" JNIEXPORT jboolean JNICALL Java_com_sanser_mobile_NativeClient_text(JNIEnv *env,jobject,jlong handle,jbyteArray utf8) {
  auto *h=reinterpret_cast<Handle*>(handle);
  if(!h || !utf8) return false;
  const auto size=env->GetArrayLength(utf8); if(size<1 || size>1024) return false;
  uint8_t data[1024]; env->GetByteArrayRegion(utf8,0,size,reinterpret_cast<jbyte*>(data));
  if(env->ExceptionCheck()) return false;
  return sanser_mobile_text(h->client,data,static_cast<size_t>(size))!=0;
}
extern "C" JNIEXPORT jboolean JNICALL Java_com_sanser_mobile_NativeClient_textAvailable(JNIEnv*,jobject,jlong handle) {
  auto *h=reinterpret_cast<Handle*>(handle); return h && sanser_mobile_text_available(h->client);
}
extern "C" JNIEXPORT jboolean JNICALL Java_com_sanser_mobile_NativeClient_gamepadCapture(JNIEnv*,jobject,jlong handle,jlong capture,jboolean enabled){
  auto* h=reinterpret_cast<Handle*>(handle);return h && capture>0 && sanser_mobile_gamepad_capture(h->client,static_cast<uint64_t>(capture),enabled);
}
extern "C" JNIEXPORT jint JNICALL Java_com_sanser_mobile_NativeClient_gamepadStatus(JNIEnv*,jobject,jlong handle,jlong capture){
  auto* h=reinterpret_cast<Handle*>(handle);return h ? sanser_mobile_gamepad_status(h->client,capture>0 ? static_cast<uint64_t>(capture):0):0;
}
extern "C" JNIEXPORT jboolean JNICALL Java_com_sanser_mobile_NativeClient_gamepadSample(JNIEnv*,jobject,jlong handle,jlong capture,jboolean connected,jboolean sampled,jint buttons,jdouble lx,jdouble ly,jdouble rx,jdouble ry,jdouble lt,jdouble rt){
  auto* h=reinterpret_cast<Handle*>(handle);if(!h || capture<=0 || buttons<0)return false;
  const SanserMobileGamepad state{connected,sampled,static_cast<uint32_t>(buttons),lx,ly,rx,ry,lt,rt};
  return sanser_mobile_gamepad_sample(h->client,static_cast<uint64_t>(capture),&state);
}
extern "C" JNIEXPORT jboolean JNICALL Java_com_sanser_mobile_NativeClient_clipboardAvailable(JNIEnv*,jobject,jlong handle) {
  auto *h=reinterpret_cast<Handle*>(handle);return h && sanser_mobile_clipboard_available(h->client);
}
extern "C" JNIEXPORT jboolean JNICALL Java_com_sanser_mobile_NativeClient_clipboard(JNIEnv *env,jobject,jlong handle,jlong request,jboolean write,jbyteArray utf8) {
  auto *h=reinterpret_cast<Handle*>(handle);if(!h || !utf8)return false;
  const auto count=env->GetArrayLength(utf8);if(count<0 || count>8192)return false;
  uint8_t text[8192];if(count)env->GetByteArrayRegion(utf8,0,count,reinterpret_cast<jbyte*>(text));if(env->ExceptionCheck())return false;
  auto call=std::make_unique<ClipboardCall>();call->owner=env->NewGlobalRef(h->owner);call->method=h->clipboard;call->request=request;
  if(!call->owner || !call->method)return false;
  // Release ownership before calling: an authenticated reply can arrive before
  // the request function returns on this thread.
  auto *context=call.release();
  if(!sanser_mobile_clipboard(h->client,write,text,static_cast<size_t>(count),context,clipboardReply)){delete context;return false;}
  return true;
}
extern "C" JNIEXPORT void JNICALL Java_com_sanser_mobile_NativeClient_keyframe(JNIEnv*,jobject,jlong handle){if(auto *h=reinterpret_cast<Handle*>(handle))sanser_mobile_keyframe(h->client);}
extern "C" JNIEXPORT jdouble JNICALL Java_com_sanser_mobile_NativeClient_rtt(JNIEnv*,jobject,jlong handle){auto *h=reinterpret_cast<Handle*>(handle);return h?sanser_mobile_rtt(h->client):-1;}
extern "C" JNIEXPORT jlongArray JNICALL Java_com_sanser_mobile_NativeClient_stats(JNIEnv *e,jobject,jlong handle){
  auto *h=reinterpret_cast<Handle*>(handle);SanserMobileStats s{};sanser_mobile_stats(h ? h->client:nullptr,&s);
  const jlong values[]={static_cast<jlong>(s.video_packets),static_cast<jlong>(s.video_frames),static_cast<jlong>(s.nack_requests),
    static_cast<jlong>(s.lost_fragments),static_cast<jlong>(s.reference_drops),static_cast<jlong>(s.audio_decoded),static_cast<jlong>(s.audio_concealed),static_cast<jlong>(s.audio_late),static_cast<jlong>(s.sample_time_ms)};
  constexpr auto count=static_cast<jsize>(sizeof(values)/sizeof(values[0]));
  auto result=e->NewLongArray(count);if(result)e->SetLongArrayRegion(result,0,count,values);return result;
}

extern "C" JNIEXPORT jstring JNICALL Java_com_sanser_mobile_MobileP2p_request(JNIEnv *env,jobject,jstring request){
  if(!request)return nullptr;
  const char *text=env->GetStringUTFChars(request,nullptr);if(!text)return nullptr;
  char *reply=sanser_mobile_p2p_request(text);env->ReleaseStringUTFChars(request,text);
  if(!reply)return nullptr;
  auto result=env->NewStringUTF(reply);sanser_mobile_p2p_free(reply);return result;
}

extern "C" JNIEXPORT void JNICALL Java_com_sanser_mobile_NativeClient_videoLimit(JNIEnv*,jobject,jlong handle,jint bitrate,jdouble baseline){if(auto *h=reinterpret_cast<Handle*>(handle))sanser_mobile_video_limit(h->client,static_cast<uint32_t>(bitrate),baseline);}

extern "C" JNIEXPORT jlong JNICALL Java_com_sanser_mobile_MobileFileBridge_create(JNIEnv* e,jobject,jbyteArray root){
  if(!root)return 0;auto n=e->GetArrayLength(root);if(n<1 || n>4096)return 0;std::vector<char> text(n+1);e->GetByteArrayRegion(root,0,n,reinterpret_cast<jbyte*>(text.data()));
  if(e->ExceptionCheck() || std::find(text.begin(),text.begin()+n,'\0')!=text.begin()+n)return 0;return static_cast<jlong>(sanser_mobile_files_create(text.data()));
}
extern "C" JNIEXPORT void JNICALL Java_com_sanser_mobile_MobileFileBridge_close(JNIEnv*,jobject,jlong id){sanser_mobile_files_destroy(static_cast<uint64_t>(id));}
extern "C" JNIEXPORT jbyteArray JNICALL Java_com_sanser_mobile_MobileFileBridge_request(JNIEnv* e,jobject,jlong id,jbyteArray raw){
  if(!raw)return nullptr;const auto n=e->GetArrayLength(raw);if(n<1 || n>4096)return nullptr;std::vector<char> text(n+1);e->GetByteArrayRegion(raw,0,n,reinterpret_cast<jbyte*>(text.data()));
  if(e->ExceptionCheck() || std::find(text.begin(),text.begin()+n,'\0')!=text.begin()+n)return nullptr;
  auto response=sanser_mobile_files_request(static_cast<uint64_t>(id),text.data());if(!response)return nullptr;
  auto result=bytes(e,reinterpret_cast<const uint8_t*>(response),std::strlen(response));sanser_mobile_p2p_free(response);return result;
}
extern "C" JNIEXPORT void JNICALL Java_com_sanser_mobile_NativeClient_filePump(JNIEnv*,jobject,jlong handle,jlong files){
  auto* h=reinterpret_cast<Handle*>(handle);if(!h || !files)return;auto c=h->client;const auto id=static_cast<uint64_t>(files);const auto epoch=sanser_mobile_file_generation(c);
  sanser_mobile_files_state(id,epoch,sanser_mobile_files_available(c));if(sanser_mobile_file_overflow(c))sanser_mobile_files_failed(id,epoch);
  uint8_t data[8192];for(unsigned i=0;i<4;++i){const auto n=sanser_mobile_file_receive(c,epoch,data,sizeof(data));if(!n)break;sanser_mobile_files_receive(id,epoch,data,n);}
  for(unsigned i=0;i<4;++i){const auto n=sanser_mobile_files_next(id,data,sizeof(data));if(n<25)break;uint32_t generation=0;for(unsigned j=0;j<4;++j)generation=(generation<<8)|data[j];if(!sanser_mobile_file_send(c,generation,data+4,n-4))sanser_mobile_files_failed(id,generation);}
}
