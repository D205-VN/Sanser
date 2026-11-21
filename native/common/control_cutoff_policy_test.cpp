#include "control_cutoff_policy.h"
#include <cstdlib>
#include <iostream>

namespace {
int failures=0;
void expect(bool ok,const char* what) { if(!ok) { std::cerr<<"FAIL: "<<what<<'\n'; ++failures; } }
}

int main() {
  using namespace sanser::cutoff;
  reset();
  expect(!revoked(),"control starts allowed");
  expect(allows(false) && allows(true),"presses and releases pass before the hotkey");
  expect(revoke(),"first press revokes");
  expect(!revoke(),"second press is not announced again");
  expect(revoked(),"stays revoked for the session");
  expect(!allows(false),"presses, moves and scrolls are dropped after the hotkey");
  expect(allows(true),"releases still pass so nothing stays held");
  reset();
  expect(allows(false),"reset restores control (next session)");
  if(failures) return EXIT_FAILURE;
  std::cout<<"control cutoff policy: ok\n";
  return EXIT_SUCCESS;
}
