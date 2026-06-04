# Check the expected output of the reporter manually

# 1. addAndSub
# - Only two files expected in the generated report:
# 	- src/TargetContract.sol
# 	- src/RaptorFuzz.sol
# - addAndSub function have 1 hit
.PHONY: check
addAndSub:
	@genhtml expected/addAndSub.info -o coverage/addAndSub --flat
	@python3 -m http.server 8000 --bind 0.0.0.0 --directory coverage/addAndSub/

# 2. addAndSub2
# - Only two files expected in the generated report:
# 	- src/TargetContract.sol
# 	- src/RaptorFuzz.sol
# - addAndSub function have 2 hits
.PHONY: check
addAndSub2:
	@genhtml expected/addAndSub2.info -o coverage/addAndSub2 --flat
	@python3 -m http.server 8000 --bind 0.0.0.0 --directory coverage/addAndSub2/

# 3. earlyReturn
# - Only two files expected in the generated report:
# 	- src/TargetContract.sol
# 	- src/RaptorFuzz.sol
# - earlyReturn function have 1 hit
.PHONY: check
earlyReturn:
	@genhtml expected/earlyReturn.info -o coverage/earlyReturn --flat
	@python3 -m http.server 8000 --bind 0.0.0.0 --directory coverage/earlyReturn/

# 4. interfaceCall
# - Only 4 files expected in the generated report:
# 	- src/TargetContract.sol
# 	- src/RaptorFuzz.sol
# 	- src/Counter.sol
# 	- src/MathLib.sol
# - interfaceCall function have 1 hit
.PHONY: check
interfaceCall:
	@genhtml expected/interfaceCall.info -o coverage/interfaceCall --flat
	@python3 -m http.server 8000 --bind 0.0.0.0 --directory coverage/interfaceCall/

# 5. libCall
# - Only 4 files expected in the generated report:
# 	- src/TargetContract.sol
# 	- src/RaptorFuzz.sol
# 	- src/Counter.sol
# 	- src/MathLib.sol
# - libCall function have 1 hit
.PHONY: check
libCall:
	@genhtml expected/libCall.info -o coverage/libCall --flat
	@python3 -m http.server 8000 --bind 0.0.0.0 --directory coverage/libCall/

# 6. libLinkedCall
# - Only 4 files expected in the generated report:
# 	- src/TargetContract.sol
# 	- src/RaptorFuzz.sol
# 	- src/CounterLinked.sol
# 	- src/MathLibLinked.sol
# - libLinkedCall function have 1 hit
.PHONY: check
libLinkedCall:
	@genhtml expected/libLinkedCall.info -o coverage/libLinkedCall --flat
	@python3 -m http.server 8000 --bind 0.0.0.0 --directory coverage/libLinkedCall/
