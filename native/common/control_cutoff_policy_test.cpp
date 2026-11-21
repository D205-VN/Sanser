#include "control_cutoff_policy.h"
#include <cstdlib>
#include <string>
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
  expect(toggle(),"first hotkey press turns control off");
  expect(revoked(),"control stays off until the host presses again");
  expect(!allows(false),"presses, moves and scrolls are dropped after the hotkey");
  expect(allows(true),"releases still pass so nothing stays held");
  expect(!toggle() && !revoked() && allows(false),"second press turns control back on");
  expect(toggle() && revoked(),"third press turns it off again");
  expect(!revoke(),"revoking while off reports no change");
  expect(std::string(notice(true))!=notice(false),"on and off notices differ");
  reset();
  expect(allows(false),"reset restores control (next session)");
  if(failures) return EXIT_FAILURE;
  std::cout<<"control cutoff policy: ok\n";
  return EXIT_SUCCESS;
}
