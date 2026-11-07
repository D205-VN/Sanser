#pragma once
#include <stdint.h>
#include <stddef.h>
#ifdef __cplusplus
extern "C" {
#endif
// UTF-8 JSON, caller frees returned memory using the paired function below.
char *sanser_mobile_p2p_request(const char *request);
void sanser_mobile_p2p_free(char *response);
// A private application workspace, shared desktop/mobile Rust file worker.
uint64_t sanser_mobile_files_create(const char *root);
void sanser_mobile_files_destroy(uint64_t handle);
void sanser_mobile_files_state(uint64_t handle,uint32_t epoch,int available);
void sanser_mobile_files_failed(uint64_t handle,uint32_t epoch);
void sanser_mobile_files_receive(uint64_t handle,uint32_t epoch,const uint8_t *data,size_t size);
size_t sanser_mobile_files_next(uint64_t handle,uint8_t *output,size_t capacity);
char *sanser_mobile_files_request(uint64_t handle,const char *request);
#ifdef __cplusplus
}
#endif
