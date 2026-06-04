# Check the expected output of the reporter manually

# 1. TargetContractBasicOnce
# - All files from TargetContractBasic compilation unit are expected:
# 	- src/TargetContractBasic.sol
# 	- src/RaptorFuzz.sol
# - addAndSub function have 1 hit
.PHONY: check
TargetContractBasicOnce:
	@genhtml expected/TargetContractBasicOnce.info -o coverage/TargetContractBasicOnce --flat
	@python3 -m http.server 8000 --bind 0.0.0.0 --directory coverage/TargetContractBasicOnce/

# 2. TargetContractBasicTwice
# - All files from TargetContractBasic compilation unit are expected:
# 	- src/TargetContractBasic.sol
# 	- src/RaptorFuzz.sol
# - addAndSub function have 2 hits
.PHONY: check
TargetContractBasicTwice:
	@genhtml expected/TargetContractBasicTwice.info -o coverage/TargetContractBasicTwice --flat
	@python3 -m http.server 8000 --bind 0.0.0.0 --directory coverage/TargetContractBasicTwice/

# 3. earlyReturn
# - All files from TargetContract compilation unit are expected:
# 	- src/TargetContract.sol
# 	- src/RaptorFuzz.sol
# 	- src/Counter.sol
# 	- src/CounterLinked.sol
# 	- src/ICounter.sol
# 	- src/MathLib.sol
# 	- src/MathLibLinked.sol
# - earlyReturn function have 1 hit
.PHONY: check
earlyReturn:
	@genhtml expected/earlyReturn.info -o coverage/earlyReturn --flat
	@python3 -m http.server 8000 --bind 0.0.0.0 --directory coverage/earlyReturn/

# 4. interfaceCall
# - All files from TargetContract compilation unit are expected:
# 	- src/TargetContract.sol
# 	- src/RaptorFuzz.sol
# 	- src/Counter.sol
# 	- src/CounterLinked.sol
# 	- src/ICounter.sol
# 	- src/MathLib.sol
# 	- src/MathLibLinked.sol
# - interfaceCall function have 1 hit
.PHONY: check
interfaceCall:
	@genhtml expected/interfaceCall.info -o coverage/interfaceCall --flat
	@python3 -m http.server 8000 --bind 0.0.0.0 --directory coverage/interfaceCall/

# 5. libCall
# - All files from TargetContract compilation unit are expected:
# 	- src/TargetContract.sol
# 	- src/RaptorFuzz.sol
# 	- src/Counter.sol
# 	- src/CounterLinked.sol
# 	- src/ICounter.sol
# 	- src/MathLib.sol
# 	- src/MathLibLinked.sol
# - libCall function have 1 hit
.PHONY: check
libCall:
	@genhtml expected/libCall.info -o coverage/libCall --flat
	@python3 -m http.server 8000 --bind 0.0.0.0 --directory coverage/libCall/

# 6. libLinkedCall
# - All files from TargetContract compilation unit are expected:
# 	- src/TargetContract.sol
# 	- src/RaptorFuzz.sol
# 	- src/Counter.sol
# 	- src/CounterLinked.sol
# 	- src/ICounter.sol
# 	- src/MathLib.sol
# 	- src/MathLibLinked.sol
# - libLinkedCall function have 1 hit
.PHONY: check
libLinkedCall:
	@genhtml expected/libLinkedCall.info -o coverage/libLinkedCall --flat
	@python3 -m http.server 8000 --bind 0.0.0.0 --directory coverage/libLinkedCall/

# 7. emptyTargetFunction
# - All files from EmptyTargetFunction compilation unit are expected:
# 	- src/EmptyTargetFunction.sol
# 	- src/RaptorFuzz.sol
# - dummyTargetFunction function have 1 hit
.PHONY: check
emptyTargetFunction:
	@genhtml expected/emptyTargetFunction.info -o coverage/emptyTargetFunction --flat
	@python3 -m http.server 8000 --bind 0.0.0.0 --directory coverage/emptyTargetFunction/
