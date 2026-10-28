#pragma once
#include <cerrno>
#ifdef _WIN32
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <winsock2.h>
#endif

namespace sanser::udp {
enum class ReceiveFailure { Idle, Retry, Fatal };

// Datagram loss is recoverable by the protocol, not by replaying arbitrary
// stale media at the socket. Invalid handles/arguments remain terminal.
inline bool transientSendError(int error) {
#ifdef _WIN32
  switch(error) {
    case WSAEINTR: case WSAEWOULDBLOCK: case WSAETIMEDOUT:
    case WSAENOBUFS: case WSAENETUNREACH: case WSAEHOSTUNREACH:
    case WSAECONNRESET: case WSAECONNREFUSED: case WSAENETRESET: case WSAENETDOWN:
      return true;
  }
#else
  switch(error) {
    case EINTR: case EAGAIN: case ETIMEDOUT:
#if EWOULDBLOCK != EAGAIN
    case EWOULDBLOCK:
#endif
    case ENOBUFS: case ENETUNREACH: case EHOSTUNREACH:
    case ECONNRESET: case ECONNREFUSED: case ENETRESET: case ENETDOWN:
      return true;
  }
#endif
  return false;
}

inline ReceiveFailure receiveFailure(int error) {
#ifdef _WIN32
  if(error==WSAEINTR || error==WSAEWOULDBLOCK || error==WSAETIMEDOUT) return ReceiveFailure::Idle;
  if(error==WSAEMSGSIZE) return ReceiveFailure::Retry; // Discard truncated datagram.
#else
  if(error==EINTR || error==EAGAIN || error==EWOULDBLOCK || error==ETIMEDOUT) return ReceiveFailure::Idle;
  if(error==EMSGSIZE) return ReceiveFailure::Retry;
#endif
  return transientSendError(error) ? ReceiveFailure::Retry : ReceiveFailure::Fatal;
}
}
