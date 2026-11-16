#include "media_thread.h"
#include <iostream>
#include <stdexcept>
#include <thread>

int main(){try{
  bool applied=false,verified=false;
  std::thread worker([&]{
#ifdef __APPLE__
    if(pthread_set_qos_class_self_np(QOS_CLASS_BACKGROUND,0)!=0)return;
#endif
    applied=sanser::prioritizeMediaThread();
#ifdef __APPLE__
    qos_class_t qos=QOS_CLASS_UNSPECIFIED;int relative=0;
    verified=pthread_get_qos_class_np(pthread_self(),&qos,&relative)==0 && qos==QOS_CLASS_USER_INTERACTIVE;
#elif defined(_WIN32)
    verified=GetThreadPriority(GetCurrentThread())==THREAD_PRIORITY_HIGHEST;
#else
    verified=true;
#endif
  });
  worker.join();
  if(!applied || !verified)throw std::runtime_error("media worker remained in inherited background QoS");
  std::cout<<"Media thread scheduling: interactive QoS on Apple, highest thread priority on Windows\n";
  return 0;
}catch(const std::exception& error){std::cerr<<error.what()<<'\n';return 1;}}
