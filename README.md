# timerqueue
My first rust library

Purpose. Allow a system to create a queue of tasks to be run asynchronously at some specified point in the future.

Client must implement the TQDispatch trait on the objects it wants tasks run on. The client can ask for a payload to be passed back to the execute function (in the trait), that payload is a std::Any, the queu simply accepts it and passes it to the object.

The client is return a handle when the task is queued, this can be queried to see if the task has finished

