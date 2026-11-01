#pragma once
#ifdef __cplusplus
extern "C" {
#endif
// UTF-8 JSON, caller frees returned memory using the paired function below.
char *sanser_mobile_p2p_request(const char *request);
void sanser_mobile_p2p_free(char *response);
#ifdef __cplusplus
}
#endif
