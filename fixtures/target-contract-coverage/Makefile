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

# 3. TargetContractWithLoop
# - All files from TargetContractWithLoop compilation unit are expected:
# 	- src/TargetContractWithLoop.sol
# 	- src/RaptorFuzz.sol
# - constructor, setup, and runLoop functions have hits
.PHONY: check
TargetContractWithLoop:
	@genhtml expected/TargetContractWithLoop.info -o coverage/TargetContractWithLoop --flat
	@python3 -m http.server 8000 --bind 0.0.0.0 --directory coverage/TargetContractWithLoop/

# 4. TargetContractWithInterface
# - All files from TargetContractWithInterface compilation unit are expected:
# 	- src/TargetContractWithInterface.sol
# 	- src/RaptorFuzz.sol
# 	- src/Counter.sol
# 	- src/ICounter.sol
# 	- src/MathLib.sol
# 	- src/Vm.sol
# - constructor and interfaceCall functions have hits
.PHONY: check
TargetContractWithInterface:
	@genhtml expected/TargetContractWithInterface.info -o coverage/TargetContractWithInterface --flat --show-zero-columns --num-spaces 4 --simplified-colors
	@python3 -m http.server 8000 --bind 0.0.0.0 --directory coverage/TargetContractWithInterface/

# 6. TargetContractWithLib
# - All files from TargetContractWithLib compilation unit are expected:
# 	- src/TargetContractWithLib.sol
# 	- src/RaptorFuzz.sol
# 	- src/Counter.sol
# 	- src/MathLib.sol
# - libCall function have 1 hit
.PHONY: check
TargetContractWithLib:
	@genhtml expected/TargetContractWithLib.info -o coverage/TargetContractWithLib --flat
	@python3 -m http.server 8000 --bind 0.0.0.0 --directory coverage/TargetContractWithLib/

# 7. TargetContractWithLibLinked
# - All files from TargetContractWithLibLinked compilation unit are expected:
# 	- src/TargetContractWithLibLinked.sol
# 	- src/RaptorFuzz.sol
# 	- src/CounterLinked.sol
# 	- src/MathLibLinked.sol
# - libLinkedCall function have 1 hit
.PHONY: check
TargetContractWithLibLinked:
	@genhtml expected/TargetContractWithLibLinked.info -o coverage/TargetContractWithLibLinked --flat
	@python3 -m http.server 8000 --bind 0.0.0.0 --directory coverage/TargetContractWithLibLinked/


